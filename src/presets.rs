use crate::params::{ExtraSynthState, PhantomSpireParams};
use crate::presets_factory::FACTORY_PRESETS_JSON;
use crate::SharedSynthState;
use nih_plug::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const PRESET_SCHEMA_VERSION: u32 = 1;
const MAX_UNDO_HISTORY: usize = 64;

pub const SPIRE_CATEGORIES: &[&str] = &[
    "Init",
    "Basses",
    "Drums",
    "FX",
    "Keys",
    "Leads",
    "Pads",
    "Plucks",
    "Seq & Arp",
    "Synth",
];

pub fn normalize_category(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::from("Synth");
    }
    for &cat in SPIRE_CATEGORIES {
        if trimmed.eq_ignore_ascii_case(cat) {
            return String::from(cat);
        }
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "init" => String::from("Init"),
        "bass" | "basses" | "acid" | "sub" => String::from("Basses"),
        "drum" | "drums" | "perc" | "percussion" => String::from("Drums"),
        "fx" | "sfx" => String::from("FX"),
        "key" | "keys" | "piano" | "organ" => String::from("Keys"),
        "lead" | "leads" => String::from("Leads"),
        "pad" | "pads" => String::from("Pads"),
        "pluck" | "plucks" => String::from("Plucks"),
        "seq & arp" | "seq&arp" | "seq" | "arp" | "arps" | "sequence" => {
            String::from("Seq & Arp")
        }
        "synth" | "synths" | "chord" | "chords" | "brass" => String::from("Synth"),
        _ => trimmed.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetBank {
    All,
    Factory,
    User,
    Favorites,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArpPatternData {
    pub steps_enabled: u16,
    pub step_offsets: [i8; 16],
    pub step_velocities: [u8; 16],
}

impl Default for ArpPatternData {
    fn default() -> Self {
        Self {
            steps_enabled: 0xFFFF,
            step_offsets: [0; 16],
            step_velocities: [100; 16],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModRouteData {
    pub source: u8,
    pub target: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MicrotuningData {
    pub preset: u8,
    pub cents: [f32; 12],
}

fn default_steppers() -> [[f32; 16]; 2] {
    [[0.5; 16]; 2]
}

fn default_schema_version() -> u32 {
    PRESET_SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetData {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub name: String,
    pub author: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub params: BTreeMap<String, serde_json::Value>,
    #[serde(default = "default_steppers")]
    pub steppers: [[f32; 16]; 2],
    pub stepper_ties: [u16; 2],
    pub arp_pattern: ArpPatternData,
    pub mod_matrix: [ModRouteData; 15],
    pub microtuning: MicrotuningData,
}

impl Default for PresetData {
    fn default() -> Self {
        Self {
            schema_version: PRESET_SCHEMA_VERSION,
            name: String::from("Init"),
            author: String::from("Phantom Spire"),
            category: String::from("Init"),
            tags: Vec::new(),
            params: BTreeMap::new(),
            steppers: default_steppers(),
            stepper_ties: [0; 2],
            arp_pattern: ArpPatternData::default(),
            mod_matrix: [ModRouteData::default(); 15],
            microtuning: MicrotuningData::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PresetEntry {
    pub name: String,
    pub author: String,
    pub category: String,
    pub tags: Vec<String>,
    pub is_factory: bool,
    pub is_favorite: bool,
    pub path: Option<PathBuf>,
    pub data: PresetData,
}

#[derive(Debug, Clone)]
pub struct PresetSnapshot {
    pub params: BTreeMap<String, serde_json::Value>,
    pub extra: ExtraSynthState,
}

pub struct PresetManager {
    user_dir: PathBuf,
    entries: Vec<PresetEntry>,
    current_index: usize,
    favorites: BTreeSet<String>,
    active_bank: PresetBank,
    active_category: Option<String>,
    active_author: Option<String>,
    active_tag: Option<String>,
    search_query: String,
    undo_stack: Vec<PresetSnapshot>,
    redo_stack: Vec<PresetSnapshot>,
}

pub fn default_user_presets_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("PhantomSpire")
        .join("Presets")
}

fn sanitize_preset_filename(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(String::from("Preset name cannot be empty"));
    }
    let mut cleaned = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_control()
            || matches!(
                ch,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        {
            cleaned.push('_');
        } else {
            cleaned.push(ch);
        }
    }
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err(String::from("Preset name contains only invalid characters"));
    }
    Ok(cleaned)
}

fn param_ptr_to_json(ptr: ParamPtr) -> serde_json::Value {
    unsafe {
        match ptr {
            ParamPtr::FloatParam(p) => serde_json::json!((*p).unmodulated_plain_value()),
            ParamPtr::IntParam(p) => serde_json::json!((*p).unmodulated_plain_value()),
            ParamPtr::BoolParam(p) => serde_json::json!((*p).unmodulated_plain_value()),
            ParamPtr::EnumParam(p) => {
                if let Some(id) = (*p).unmodulated_plain_id() {
                    serde_json::json!(id)
                } else {
                    serde_json::json!((*p).unmodulated_plain_value())
                }
            }
        }
    }
}

fn json_value_to_normalized(ptr: ParamPtr, val: &serde_json::Value) -> Option<f32> {
    unsafe {
        match ptr {
            ParamPtr::FloatParam(p) => val
                .as_f64()
                .filter(|v| v.is_finite())
                .map(|v| (*p).preview_normalized(v as f32)),
            ParamPtr::IntParam(p) => val
                .as_i64()
                .or_else(|| val.as_f64().map(|v| v.round() as i64))
                .map(|v| (*p).preview_normalized(v as i32)),
            ParamPtr::BoolParam(p) => val
                .as_bool()
                .or_else(|| val.as_i64().map(|v| v != 0))
                .map(|b| (*p).preview_normalized(b)),
            ParamPtr::EnumParam(p) => {
                if let Some(s) = val.as_str() {
                    (*p).string_to_normalized_value(s)
                } else {
                    val.as_i64().map(|idx| (*p).preview_normalized(idx as i32))
                }
            }
        }
    }
}

fn apply_preset_data_to_extra(
    entry_name: &str,
    entry_category: &str,
    data: &PresetData,
    extra: &mut ExtraSynthState,
) {
    extra.preset_name = entry_name.to_string();
    extra.preset_category = entry_category.to_string();
    extra.preset_author = data.author.clone();
    extra.stepper_steps = data.steppers;
    extra.stepper_ties = data.stepper_ties;
    extra.arp_steps_enabled = data.arp_pattern.steps_enabled;
    extra.arp_step_offsets = data.arp_pattern.step_offsets;
    extra.arp_step_velocities = data.arp_pattern.step_velocities;
    for i in 0..15 {
        extra.mod_sources[i] = data.mod_matrix[i].source;
        extra.mod_targets[i] = data.mod_matrix[i].target;
    }
    extra.microtuning_preset = data.microtuning.preset;
    extra.microtuning_cents = data.microtuning.cents;
}

impl Default for PresetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PresetManager {
    pub fn new() -> Self {
        Self::with_user_dir(default_user_presets_dir())
    }

    pub fn with_user_dir(user_dir: PathBuf) -> Self {
        let mut mgr = Self {
            user_dir,
            entries: Vec::new(),
            current_index: 0,
            favorites: BTreeSet::new(),
            active_bank: PresetBank::All,
            active_category: None,
            active_author: None,
            active_tag: None,
            search_query: String::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        };
        mgr.load_favorites();
        mgr.rescan();
        mgr
    }

    pub fn user_dir(&self) -> &PathBuf {
        &self.user_dir
    }

    fn favorites_path(&self) -> PathBuf {
        self.user_dir.join(".favorites.json")
    }

    fn load_favorites(&mut self) {
        let path = self.favorites_path();
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) {
                self.favorites = list.into_iter().collect();
            }
        }
    }

    fn save_favorites(&self) {
        let _ = std::fs::create_dir_all(&self.user_dir);
        let list: Vec<&String> = self.favorites.iter().collect();
        if let Ok(json) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(self.favorites_path(), json);
        }
    }

    fn favorite_key(is_factory: bool, name: &str) -> String {
        if is_factory {
            format!("factory:{name}")
        } else {
            format!("user:{name}")
        }
    }

    pub fn rescan(&mut self) {
        let previous_name = self.entries.get(self.current_index).map(|e| e.name.clone());
        let mut entries = Vec::with_capacity(FACTORY_PRESETS_JSON.len());

        for (filename, raw_json) in FACTORY_PRESETS_JSON {
            match serde_json::from_str::<PresetData>(raw_json) {
                Ok(data) => {
                    let name = if data.name.trim().is_empty() {
                        filename.trim_end_matches(".json").to_string()
                    } else {
                        data.name.clone()
                    };
                    let category = normalize_category(&data.category);
                    let author = if data.author.trim().is_empty() {
                        String::from("Phantom Spire")
                    } else {
                        data.author.clone()
                    };
                    let tags = data.tags.clone();
                    let is_favorite = self.favorites.contains(&Self::favorite_key(true, &name));
                    entries.push(PresetEntry {
                        name,
                        author,
                        category,
                        tags,
                        is_factory: true,
                        is_favorite,
                        path: None,
                        data,
                    });
                }
                Err(err) => {
                    nih_plug::nih_error!("Failed to parse factory preset {filename}: {err}");
                }
            }
        }

        if entries.is_empty() {
            entries.push(PresetEntry {
                name: String::from("Init"),
                author: String::from("Phantom Spire"),
                category: String::from("Init"),
                tags: Vec::new(),
                is_factory: true,
                is_favorite: false,
                path: None,
                data: PresetData::default(),
            });
        }

        if self.user_dir.is_dir() {
            if let Ok(read_dir) = std::fs::read_dir(&self.user_dir) {
                let mut user_paths: Vec<PathBuf> = read_dir
                    .filter_map(|e| e.ok().map(|entry| entry.path()))
                    .filter(|p| {
                        let is_hidden = p
                            .file_name()
                            .and_then(|s| s.to_str())
                            .map(|s| s.starts_with('.'))
                            .unwrap_or(false);
                        !is_hidden
                            && p.is_file()
                            && p.extension()
                                .and_then(|s| s.to_str())
                                .map(|s| s.eq_ignore_ascii_case("json"))
                                .unwrap_or(false)
                    })
                    .collect();
                user_paths.sort();

                for path in user_paths {
                    let Ok(contents) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    let Ok(data) = serde_json::from_str::<PresetData>(&contents) else {
                        continue;
                    };
                    let fallback_name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("User Preset")
                        .to_string();
                    let name = if data.name.trim().is_empty() {
                        fallback_name
                    } else {
                        data.name.clone()
                    };
                    let category = if data.category.trim().is_empty() {
                        String::from("Synth")
                    } else {
                        normalize_category(&data.category)
                    };
                    let author = if data.author.trim().is_empty() {
                        String::from("User")
                    } else {
                        data.author.clone()
                    };
                    let tags = data.tags.clone();
                    let is_favorite = self.favorites.contains(&Self::favorite_key(false, &name));
                    entries.push(PresetEntry {
                        name,
                        author,
                        category,
                        tags,
                        is_factory: false,
                        is_favorite,
                        path: Some(path),
                        data,
                    });
                }
            }
        }

        self.entries = entries;
        if let Some(prev) = previous_name {
            if let Some(idx) = self.entries.iter().position(|e| e.name == prev) {
                self.current_index = idx;
                return;
            }
        }
        self.current_index = 0;
    }

    pub fn entries(&self) -> &[PresetEntry] {
        &self.entries
    }

    pub fn current_index(&self) -> usize {
        self.current_index
    }

    pub fn current_preset(&self) -> &PresetEntry {
        &self.entries[self.current_index]
    }

    pub fn active_bank(&self) -> PresetBank {
        self.active_bank
    }

    pub fn set_active_bank(&mut self, bank: PresetBank) {
        self.active_bank = bank;
    }

    pub fn active_category(&self) -> Option<&str> {
        self.active_category.as_deref()
    }

    pub fn set_active_category(&mut self, category: Option<&str>) {
        self.active_category = category.and_then(|c| {
            let trimmed = c.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("ALL") {
                None
            } else {
                Some(normalize_category(trimmed))
            }
        });
    }

    pub fn active_author(&self) -> Option<&str> {
        self.active_author.as_deref()
    }

    pub fn set_active_author(&mut self, author: Option<&str>) {
        self.active_author = author.and_then(|a| {
            let trimmed = a.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("ALL") {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    }

    pub fn active_tag(&self) -> Option<&str> {
        self.active_tag.as_deref()
    }

    pub fn set_active_tag(&mut self, tag: Option<&str>) {
        self.active_tag = tag.and_then(|t| {
            let trimmed = t.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("ALL") {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn set_search_query(&mut self, query: &str) {
        self.search_query = query.to_string();
    }

    pub fn is_favorite(&self, index: usize) -> bool {
        self.entries.get(index).map(|e| e.is_favorite).unwrap_or(false)
    }

    pub fn set_favorite(&mut self, index: usize, fav: bool) -> bool {
        let Some(entry) = self.entries.get_mut(index) else {
            return false;
        };
        let key = Self::favorite_key(entry.is_factory, &entry.name);
        entry.is_favorite = fav;
        if fav {
            self.favorites.insert(key);
        } else {
            self.favorites.remove(&key);
        }
        self.save_favorites();
        fav
    }

    pub fn toggle_favorite(&mut self, index: usize) -> bool {
        let next = !self.is_favorite(index);
        self.set_favorite(index, next)
    }

    fn matches_filter(
        entry: &PresetEntry,
        bank: PresetBank,
        category: Option<&str>,
        author: Option<&str>,
        tag: Option<&str>,
        search_lower: &str,
    ) -> bool {
        let bank_ok = match bank {
            PresetBank::All => true,
            PresetBank::Factory => entry.is_factory,
            PresetBank::User => !entry.is_factory,
            PresetBank::Favorites => entry.is_favorite,
        };
        if !bank_ok {
            return false;
        }
        if let Some(cat) = category {
            if !cat.is_empty() && !cat.eq_ignore_ascii_case("ALL") {
                let norm = normalize_category(cat);
                if !entry.category.eq_ignore_ascii_case(&norm) {
                    return false;
                }
            }
        }
        if let Some(auth) = author {
            if !auth.is_empty() && !entry.author.eq_ignore_ascii_case(auth) {
                return false;
            }
        }
        if let Some(t) = tag {
            if !t.is_empty() && !entry.tags.iter().any(|et| et.eq_ignore_ascii_case(t)) {
                return false;
            }
        }
        if !search_lower.is_empty() {
            let name_hit = entry.name.to_ascii_lowercase().contains(search_lower);
            let cat_hit = entry.category.to_ascii_lowercase().contains(search_lower);
            let auth_hit = entry.author.to_ascii_lowercase().contains(search_lower);
            let tag_hit = entry
                .tags
                .iter()
                .any(|t| t.to_ascii_lowercase().contains(search_lower));
            if !name_hit && !cat_hit && !auth_hit && !tag_hit {
                return false;
            }
        }
        true
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        self.filtered_indices_query(
            self.active_bank,
            self.active_category.as_deref(),
            self.active_author.as_deref(),
            self.active_tag.as_deref(),
            &self.search_query,
        )
    }

    pub fn filtered_indices_for(
        &self,
        bank: PresetBank,
        category: Option<&str>,
        search: &str,
    ) -> Vec<usize> {
        self.filtered_indices_query(bank, category, None, None, search)
    }

    pub fn filtered_indices_query(
        &self,
        bank: PresetBank,
        category: Option<&str>,
        author: Option<&str>,
        tag: Option<&str>,
        search: &str,
    ) -> Vec<usize> {
        let search_lower = search.trim().to_ascii_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| Self::matches_filter(e, bank, category, author, tag, &search_lower))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn category_counts(&self, bank: PresetBank, search: &str) -> Vec<(&'static str, usize)> {
        let search_lower = search.trim().to_ascii_lowercase();
        SPIRE_CATEGORIES
            .iter()
            .map(|&cat| {
                let count = self
                    .entries
                    .iter()
                    .filter(|e| Self::matches_filter(e, bank, Some(cat), None, None, &search_lower))
                    .count();
                (cat, count)
            })
            .collect()
    }

    pub fn authors(&self, bank: PresetBank) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for e in &self.entries {
            if Self::matches_filter(e, bank, None, None, None, "") {
                *counts.entry(e.author.clone()).or_insert(0) += 1;
            }
        }
        counts.into_iter().collect()
    }

    pub fn tags(&self, bank: PresetBank) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for e in &self.entries {
            if Self::matches_filter(e, bank, None, None, None, "") {
                for t in &e.tags {
                    *counts.entry(t.clone()).or_insert(0) += 1;
                }
            }
        }
        counts.into_iter().collect()
    }

    pub fn total_count_for_bank(&self, bank: PresetBank) -> usize {
        self.entries
            .iter()
            .filter(|e| Self::matches_filter(e, bank, None, None, None, ""))
            .count()
    }

    pub fn select_next(&mut self) -> usize {
        let bank = self.active_bank;
        let cat = self.active_category.clone();
        let auth = self.active_author.clone();
        let tag = self.active_tag.clone();
        let search = self.search_query.clone();
        let filtered = self.filtered_indices_query(
            bank,
            cat.as_deref(),
            auth.as_deref(),
            tag.as_deref(),
            &search,
        );
        self.step_next_in(&filtered)
    }

    pub fn select_prev(&mut self) -> usize {
        let bank = self.active_bank;
        let cat = self.active_category.clone();
        let auth = self.active_author.clone();
        let tag = self.active_tag.clone();
        let search = self.search_query.clone();
        let filtered = self.filtered_indices_query(
            bank,
            cat.as_deref(),
            auth.as_deref(),
            tag.as_deref(),
            &search,
        );
        self.step_prev_in(&filtered)
    }

    fn step_next_in(&mut self, filtered: &[usize]) -> usize {
        if filtered.is_empty() {
            if !self.entries.is_empty() {
                self.current_index = (self.current_index + 1) % self.entries.len();
            }
            return self.current_index;
        }
        if let Some(pos) = filtered.iter().position(|&idx| idx == self.current_index) {
            self.current_index = filtered[(pos + 1) % filtered.len()];
        } else {
            self.current_index = filtered[0];
        }
        self.current_index
    }

    fn step_prev_in(&mut self, filtered: &[usize]) -> usize {
        if filtered.is_empty() {
            if !self.entries.is_empty() {
                self.current_index = if self.current_index == 0 {
                    self.entries.len() - 1
                } else {
                    self.current_index - 1
                };
            }
            return self.current_index;
        }
        if let Some(pos) = filtered.iter().position(|&idx| idx == self.current_index) {
            self.current_index = if pos == 0 {
                filtered[filtered.len() - 1]
            } else {
                filtered[pos - 1]
            };
        } else {
            self.current_index = filtered[filtered.len() - 1];
        }
        self.current_index
    }

    pub fn select_next_filtered(
        &mut self,
        bank: PresetBank,
        category: Option<&str>,
        search: &str,
    ) -> usize {
        let filtered = self.filtered_indices_for(bank, category, search);
        self.step_next_in(&filtered)
    }

    pub fn select_prev_filtered(
        &mut self,
        bank: PresetBank,
        category: Option<&str>,
        search: &str,
    ) -> usize {
        let filtered = self.filtered_indices_for(bank, category, search);
        self.step_prev_in(&filtered)
    }

    pub fn save_user_preset_with_category(
        &mut self,
        name: &str,
        category: &str,
        data: PresetData,
    ) -> Result<PathBuf, String> {
        self.save_user_preset(name, category, data)
    }

    pub fn save_user_preset(
        &mut self,
        name: &str,
        category: &str,
        mut data: PresetData,
    ) -> Result<PathBuf, String> {
        let clean_stem = sanitize_preset_filename(name)?;
        std::fs::create_dir_all(&self.user_dir)
            .map_err(|e| format!("Failed to create preset directory: {e}"))?;

        data.schema_version = PRESET_SCHEMA_VERSION;
        data.name = name.trim().to_string();
        if !category.trim().is_empty() {
            data.category = normalize_category(category);
        } else if data.category.trim().is_empty() {
            data.category = String::from("Synth");
        } else {
            data.category = normalize_category(&data.category);
        }
        if data.author.trim().is_empty() || data.author == "Phantom Spire" {
            data.author = String::from("User");
        }

        let target_path = self.user_dir.join(format!("{clean_stem}.json"));
        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| format!("Failed to serialize preset JSON: {e}"))?;
        std::fs::write(&target_path, json)
            .map_err(|e| format!("Failed to write preset file: {e}"))?;

        self.rescan();
        if let Some(idx) = self
            .entries
            .iter()
            .position(|e| e.path.as_ref() == Some(&target_path))
        {
            self.current_index = idx;
        }
        Ok(target_path)
    }

    pub fn delete_user_preset(&mut self, index: usize) -> Result<(), String> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| format!("Preset index {index} out of bounds"))?;
        if entry.is_factory {
            return Err(String::from("Cannot delete embedded factory preset"));
        }
        let path = entry
            .path
            .clone()
            .ok_or_else(|| String::from("User preset has no backing file path"))?;
        std::fs::remove_file(&path).map_err(|e| format!("Failed to remove preset: {e}"))?;
        self.rescan();
        if self.current_index >= self.entries.len() {
            self.current_index = self.entries.len().saturating_sub(1);
        }
        Ok(())
    }

    pub fn push_undo_snapshot(&mut self, snapshot: PresetSnapshot) {
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
        self.redo_stack.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn capture_param_values(params: &PhantomSpireParams) -> BTreeMap<String, serde_json::Value> {
        params
            .param_map()
            .into_iter()
            .map(|(id, ptr, _)| (id, param_ptr_to_json(ptr)))
            .collect()
    }

    pub fn capture_from_params(params: &PhantomSpireParams) -> PresetData {
        let extra = params
            .extra_state
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let mut mod_matrix = [ModRouteData::default(); 15];
        for i in 0..15 {
            mod_matrix[i] = ModRouteData {
                source: extra.mod_sources[i],
                target: extra.mod_targets[i],
            };
        }
        PresetData {
            schema_version: PRESET_SCHEMA_VERSION,
            name: extra.preset_name.clone(),
            author: extra.preset_author.clone(),
            category: extra.preset_category.clone(),
            tags: Vec::new(),
            params: Self::capture_param_values(params),
            steppers: extra.stepper_steps,
            stepper_ties: extra.stepper_ties,
            arp_pattern: ArpPatternData {
                steps_enabled: extra.arp_steps_enabled,
                step_offsets: extra.arp_step_offsets,
                step_velocities: extra.arp_step_velocities,
            },
            mod_matrix,
            microtuning: MicrotuningData {
                preset: extra.microtuning_preset,
                cents: extra.microtuning_cents,
            },
        }
    }

    fn capture_snapshot_from_params(params: &PhantomSpireParams) -> PresetSnapshot {
        let extra = params
            .extra_state
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        PresetSnapshot {
            params: Self::capture_param_values(params),
            extra,
        }
    }

    fn apply_snapshot_to_gui(
        snapshot: &PresetSnapshot,
        params: &PhantomSpireParams,
        setter: &ParamSetter,
        shared_synth: &SharedSynthState,
        reset_missing_to_default: bool,
    ) {
        if let Ok(mut guard) = params.extra_state.lock() {
            *guard = snapshot.extra.clone();
            shared_synth.sync_from_extra(&guard);
        }

        for (id, ptr, _group) in params.param_map() {
            let target_norm = match snapshot.params.get(&id) {
                Some(v) => match json_value_to_normalized(ptr, v) {
                    Some(norm) => norm,
                    None => continue,
                },
                None if reset_missing_to_default => unsafe { ptr.default_normalized_value() },
                None => continue,
            };

            let current_norm = unsafe { ptr.unmodulated_normalized_value() };
            let target_clamped = target_norm.clamp(0.0, 1.0);
            if (target_clamped - current_norm).abs() > 1e-6 {
                unsafe {
                    setter.raw_context.raw_begin_set_parameter(ptr);
                    setter
                        .raw_context
                        .raw_set_parameter_normalized(ptr, target_clamped);
                    setter.raw_context.raw_end_set_parameter(ptr);
                }
            }
        }
    }

    pub fn apply_to_gui(
        &mut self,
        index: usize,
        params: &PhantomSpireParams,
        setter: &ParamSetter,
        shared_synth: &SharedSynthState,
    ) -> Result<(), String> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| format!("Preset index {index} out of bounds"))?
            .clone();

        let current_snapshot = Self::capture_snapshot_from_params(params);
        self.push_undo_snapshot(current_snapshot);
        self.current_index = index;

        let mut next_extra = params
            .extra_state
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        apply_preset_data_to_extra(&entry.name, &entry.category, &entry.data, &mut next_extra);

        let target_snapshot = PresetSnapshot {
            params: entry.data.params.clone(),
            extra: next_extra,
        };
        Self::apply_snapshot_to_gui(&target_snapshot, params, setter, shared_synth, true);
        Ok(())
    }

    pub fn undo_gui(
        &mut self,
        params: &PhantomSpireParams,
        setter: &ParamSetter,
        shared_synth: &SharedSynthState,
    ) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        let current = Self::capture_snapshot_from_params(params);
        self.redo_stack.push(current);
        Self::apply_snapshot_to_gui(&prev, params, setter, shared_synth, true);
        true
    }

    pub fn redo_gui(
        &mut self,
        params: &PhantomSpireParams,
        setter: &ParamSetter,
        shared_synth: &SharedSynthState,
    ) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let current = Self::capture_snapshot_from_params(params);
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(current);
        Self::apply_snapshot_to_gui(&next, params, setter, shared_synth, true);
        true
    }

    pub fn apply_to_headless(
        &mut self,
        index: usize,
        param_map: &mut BTreeMap<String, serde_json::Value>,
        extra: &mut ExtraSynthState,
        shared_synth: &SharedSynthState,
    ) -> Result<(), String> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| format!("Preset index {index} out of bounds"))?
            .clone();

        self.push_undo_snapshot(PresetSnapshot {
            params: param_map.clone(),
            extra: extra.clone(),
        });
        self.current_index = index;

        let defaults = PhantomSpireParams::default();
        *param_map = Self::capture_param_values(&defaults);
        for (k, v) in &entry.data.params {
            param_map.insert(k.clone(), v.clone());
        }
        apply_preset_data_to_extra(&entry.name, &entry.category, &entry.data, extra);
        shared_synth.sync_from_extra(extra);
        Ok(())
    }

    pub fn undo_headless(
        &mut self,
        param_map: &mut BTreeMap<String, serde_json::Value>,
        extra: &mut ExtraSynthState,
        shared_synth: &SharedSynthState,
    ) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(PresetSnapshot {
            params: param_map.clone(),
            extra: extra.clone(),
        });
        *param_map = prev.params;
        *extra = prev.extra;
        shared_synth.sync_from_extra(extra);
        true
    }

    pub fn redo_headless(
        &mut self,
        param_map: &mut BTreeMap<String, serde_json::Value>,
        extra: &mut ExtraSynthState,
        shared_synth: &SharedSynthState,
    ) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(PresetSnapshot {
            params: param_map.clone(),
            extra: extra.clone(),
        });
        *param_map = next.params;
        *extra = next.extra;
        shared_synth.sync_from_extra(extra);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{SynthEngine, TransportInfo};
    use crate::params::{
        set_bool_param, set_enum_param, set_float_param, set_int_param, FilterModel, FilterType,
        OscMode, OscWaveform, PolyMode, ShaperMode,
    };
    use crate::SharedUiState;
    use std::collections::{HashMap, HashSet};

    fn temp_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phantom_spire_preset_test_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn apply_key_sound_params(data: &PresetData, params: &mut PhantomSpireParams) {
        let get_f = |k: &str| data.params.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
        let get_i = |k: &str| data.params.get(k).and_then(|v| v.as_i64()).map(|v| v as i32);
        let get_b = |k: &str| data.params.get(k).and_then(|v| v.as_bool());
        let get_s = |k: &str| data.params.get(k).and_then(|v| v.as_str());

        if let Some("Mono 1") = get_s("g_pmod") {
            set_enum_param(&mut params.global.poly_mode, PolyMode::Mono1);
        } else if let Some("Mono 2") = get_s("g_pmod") {
            set_enum_param(&mut params.global.poly_mode, PolyMode::Mono2);
        } else if let Some("Legato") = get_s("g_pmod") {
            set_enum_param(&mut params.global.poly_mode, PolyMode::Legato);
        }
        if let Some(v) = get_b("a_on") {
            set_bool_param(&mut params.arp.enabled, v);
        }
        for i in 0..4 {
            let n = i + 1;
            if let Some(lvl) = get_f(&format!("o_lvl_{n}")) {
                set_float_param(&mut params.oscs[i].mix_level, lvl);
            }
            if let Some(oct) = get_i(&format!("o_oct_{n}")) {
                set_int_param(&mut params.oscs[i].octave, oct);
            }
            if let Some(uc) = get_i(&format!("o_ucnt_{n}")) {
                set_int_param(&mut params.oscs[i].unison_count, uc);
            }
            if let Some(m) = get_s(&format!("o_mod_{n}")) {
                let mode = match m {
                    "Noise" => OscMode::Noise,
                    "FM" => OscMode::FM,
                    "AMSync" => OscMode::AMSync,
                    "SawPWM" => OscMode::SawPWM,
                    "HardFM" => OscMode::HardFM,
                    "Vowel" => OscMode::Vowel,
                    _ => OscMode::Classic,
                };
                set_enum_param(&mut params.oscs[i].mode, mode);
            }
            if let Some(w) = get_s(&format!("o_wav_{n}")) {
                let wav = match w {
                    "Sin" => OscWaveform::Sin,
                    "Tri" => OscWaveform::Tri,
                    "Square" => OscWaveform::Square,
                    "Pulse" => OscWaveform::Pulse,
                    "SuperSaw" => OscWaveform::SuperSaw,
                    "Organ" => OscWaveform::Organ,
                    "Bell" => OscWaveform::Bell,
                    "Formant" => OscWaveform::Formant,
                    "Metallic" => OscWaveform::Metallic,
                    "Acid" => OscWaveform::Acid,
                    "Bass" => OscWaveform::Bass,
                    "Choir" => OscWaveform::Choir,
                    "Strings" => OscWaveform::Strings,
                    "Hollow" => OscWaveform::Hollow,
                    "Sub" => OscWaveform::Sub,
                    _ => OscWaveform::Saw,
                };
                set_enum_param(&mut params.oscs[i].waveform, wav);
            }
        }
        for i in 0..2 {
            let n = i + 1;
            if let Some(cut) = get_f(&format!("f_cut_{n}")) {
                set_float_param(&mut params.filters[i].cutoff, cut);
            }
            if let Some(res) = get_f(&format!("f_res_{n}")) {
                set_float_param(&mut params.filters[i].resonance, res);
            }
            if let Some(m) = get_s(&format!("f_mod_{n}")) {
                let model = match m {
                    "Infecto" => FilterModel::Infecto,
                    "Acido" => FilterModel::Acido,
                    "Scorpio" => FilterModel::Scorpio,
                    "Combo" => FilterModel::Combo,
                    _ => FilterModel::Perfecto,
                };
                set_enum_param(&mut params.filters[i].model, model);
            }
            if let Some(t) = get_s(&format!("f_typ_{n}")) {
                let ft = match t {
                    "LP2" => FilterType::LP2,
                    "LR4" => FilterType::LR4,
                    "BP2" => FilterType::BP2,
                    "BP4" => FilterType::BP4,
                    "HP2" => FilterType::HP2,
                    "HP4" => FilterType::HP4,
                    "Peak" => FilterType::Peak,
                    "Notch" => FilterType::Notch,
                    _ => FilterType::LP4,
                };
                set_enum_param(&mut params.filters[i].filter_type, ft);
            }
        }
        if let Some(att) = get_f("e_att_1") {
            set_float_param(&mut params.envs[0].attack, att);
        }
        if let Some(dec) = get_f("e_dec_1") {
            set_float_param(&mut params.envs[0].decay, dec);
        }
        if let Some(sus) = get_f("e_sus_1") {
            set_float_param(&mut params.envs[0].sustain, sus);
        }
        if let Some(s) = get_s("x_smod") {
            let sm = match s {
                "Soft" => ShaperMode::Soft,
                "Hard" => ShaperMode::Hard,
                "Clip" => ShaperMode::Clip,
                "Tube" => ShaperMode::Tube,
                "Fold" => ShaperMode::Fold,
                "Sine" => ShaperMode::Sine,
                "Bitcrush" => ShaperMode::Bitcrush,
                _ => ShaperMode::Warm,
            };
            set_enum_param(&mut params.fx.shaper.mode, sm);
        }
        if let Some(v) = get_f("x_sdw") {
            set_float_param(&mut params.fx.shaper.dry_wet, v);
        }
        if let Some(v) = get_f("x_rdw") {
            set_float_param(&mut params.fx.reverb.dry_wet, v);
        }
    }

    #[test]
    fn all_factory_presets_parse_match_params_and_render_finite_audio() {
        assert_eq!(FACTORY_PRESETS_JSON.len(), 291);

        let defaults = PhantomSpireParams::default();
        let param_map: HashMap<String, ParamPtr> = defaults
            .param_map()
            .into_iter()
            .map(|(id, ptr, _)| (id, ptr))
            .collect();

        let mut seen_categories = HashSet::new();
        let mut seen_names = HashSet::new();

        for (idx, (filename, raw)) in FACTORY_PRESETS_JSON.iter().enumerate() {
            let preset: PresetData = serde_json::from_str(raw)
                .unwrap_or_else(|e| panic!("Factory preset {filename} failed to parse: {e}"));
            assert_eq!(preset.schema_version, PRESET_SCHEMA_VERSION);
            assert!(!preset.name.trim().is_empty(), "{filename} empty name");
            assert!(!preset.category.trim().is_empty(), "{filename} empty category");
            assert!(
                seen_names.insert(preset.name.clone()),
                "Duplicate factory preset name '{}'",
                preset.name
            );
            seen_categories.insert(normalize_category(&preset.category));

            for (key, val) in &preset.params {
                let ptr = param_map.get(key).unwrap_or_else(|| {
                    panic!("Factory preset '{filename}' contains unknown param key '{key}'")
                });
                let norm = json_value_to_normalized(*ptr, val).unwrap_or_else(|| {
                    panic!(
                        "Factory preset '{filename}' key '{key}' value {val:?} failed conversion"
                    )
                });
                assert!(
                    (0.0..=1.0).contains(&norm) && norm.is_finite(),
                    "Factory preset '{filename}' key '{key}' normalized out of range: {norm}"
                );
            }

            let mut params = PhantomSpireParams::default();
            apply_key_sound_params(&preset, &mut params);
            let shared = SharedSynthState::default();
            let mut extra = ExtraSynthState::default();
            apply_preset_data_to_extra(&preset.name, &preset.category, &preset, &mut extra);
            shared.sync_from_extra(&extra);
            let ui = SharedUiState::default();
            let transport = TransportInfo::default();
            let mut engine = SynthEngine::new(44100.0);

            engine.note_on(60, 0.85, 0);
            let mut ch_l = vec![0.0_f32; 2048];
            let mut ch_r = vec![0.0_f32; 2048];
            let mut buf = Buffer::default();
            unsafe {
                buf.set_slices(2048, |s| {
                    s.clear();
                    s.push(&mut ch_l);
                    s.push(&mut ch_r);
                });
            }
            engine.process(&mut buf, &params, &shared, &ui, &transport);
            assert!(
                ch_l.iter().all(|s| s.is_finite() && s.abs() <= 0.995)
                    && ch_r.iter().all(|s| s.is_finite() && s.abs() <= 0.995),
                "Preset [{idx}] '{}' produced non-finite or clipped audio",
                preset.name
            );
            let energy: f32 = ch_l.iter().chain(ch_r.iter()).map(|s| s * s).sum();
            let rms = (energy / (2.0 * 2048.0)).sqrt();
            assert!(
                rms > 0.0005,
                "Preset [{idx}] '{}' produced silent audio (rms={rms})",
                preset.name
            );
        }

        for &cat in SPIRE_CATEGORIES {
            assert!(
                seen_categories.contains(cat),
                "Missing factory presets for category '{cat}'"
            );
        }
    }

    #[test]
    fn user_preset_save_load_delete_and_wrap_navigation() {
        let dir = temp_test_dir("crud");
        let mut mgr = PresetManager::with_user_dir(dir.clone());
        let factory_count = mgr.entries().len();
        assert_eq!(factory_count, 291);
        assert_eq!(mgr.current_index(), 0);
        assert_eq!(mgr.current_preset().name, "Init");

        // Wrap prev from 0 -> 290
        assert_eq!(mgr.select_prev(), factory_count - 1);
        // Wrap next from 290 -> 0
        assert_eq!(mgr.select_next(), 0);

        // Factory preset cannot be deleted
        assert!(mgr.delete_user_preset(0).is_err());

        // Save user preset with dirty characters sanitized & legacy category normalized
        let params = PhantomSpireParams::default();
        let mut data = PresetManager::capture_from_params(&params);
        data.params
            .insert(String::from("f_cut_1"), serde_json::json!(1234.0));
        let saved_path = mgr
            .save_user_preset("My/Cool:Bass?", "Bass", data)
            .expect("save user preset");
        assert!(saved_path.exists());
        assert_eq!(mgr.entries().len(), factory_count + 1);

        // Reload in fresh manager and verify it's loaded with normalized category
        let mut mgr2 = PresetManager::with_user_dir(dir.clone());
        assert_eq!(mgr2.entries().len(), factory_count + 1);
        let user_idx = mgr2
            .entries()
            .iter()
            .position(|e| !e.is_factory)
            .expect("user preset present");
        assert_eq!(mgr2.entries()[user_idx].category, "Basses");
        assert_eq!(
            mgr2.entries()[user_idx].data.params.get("f_cut_1"),
            Some(&serde_json::json!(1234.0))
        );

        // Delete user preset
        mgr2.delete_user_preset(user_idx).expect("delete user preset");
        assert_eq!(mgr2.entries().len(), factory_count);
        assert!(!saved_path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preset_browser_filtering_categories_authors_tags_search_and_favorites_work() {
        let dir = temp_test_dir("browser");
        let mut mgr = PresetManager::with_user_dir(dir.clone());

        assert_eq!(mgr.total_count_for_bank(PresetBank::All), 291);
        assert_eq!(mgr.total_count_for_bank(PresetBank::Factory), 291);
        assert_eq!(mgr.total_count_for_bank(PresetBank::User), 0);
        assert_eq!(mgr.total_count_for_bank(PresetBank::Favorites), 0);

        // Category filter + legacy normalization ("Bass" -> "Basses")
        let counts = mgr.category_counts(PresetBank::All, "");
        assert_eq!(counts.len(), SPIRE_CATEGORIES.len());
        let bass_count = counts.iter().find(|(c, _)| *c == "Basses").unwrap().1;
        assert_eq!(bass_count, 46);

        mgr.set_active_category(Some("Bass"));
        assert_eq!(mgr.active_category(), Some("Basses"));
        let bass_indices = mgr.filtered_indices();
        assert_eq!(bass_indices.len(), 46);
        assert!(bass_indices
            .iter()
            .all(|&i| mgr.entries()[i].category == "Basses"));

        // Authors & Tags
        let authors = mgr.authors(PresetBank::Factory);
        assert!(authors.len() >= 5);
        let tags = mgr.tags(PresetBank::Factory);
        assert!(tags.len() >= 10);

        // Search filter
        mgr.set_active_category(None);
        mgr.set_search_query("303");
        let search_hits = mgr.filtered_indices();
        assert!(!search_hits.is_empty());

        // Favorites toggle + reload persistence
        mgr.set_search_query("");
        assert!(mgr.toggle_favorite(2));
        assert_eq!(mgr.total_count_for_bank(PresetBank::Favorites), 1);

        let mgr_reloaded = PresetManager::with_user_dir(dir.clone());
        assert_eq!(mgr_reloaded.total_count_for_bank(PresetBank::Favorites), 1);
        assert!(mgr_reloaded.entries()[2].is_favorite);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn headless_apply_and_undo_redo_work_correctly() {
        let dir = temp_test_dir("undo");
        let mut mgr = PresetManager::with_user_dir(dir.clone());
        let defaults = PhantomSpireParams::default();
        let mut param_map = PresetManager::capture_param_values(&defaults);
        let mut extra = ExtraSynthState::default();
        let shared_synth = SharedSynthState::default();

        assert!(!mgr.can_undo());
        assert!(!mgr.can_redo());

        let hyper_idx = mgr
            .entries()
            .iter()
            .position(|e| e.name == "LD Crimson Hyper")
            .expect("LD Crimson Hyper present");
        mgr.apply_to_headless(hyper_idx, &mut param_map, &mut extra, &shared_synth)
            .expect("apply LD Crimson Hyper");
        assert_eq!(extra.preset_name, "LD Crimson Hyper");
        assert_eq!(param_map.get("o_wav_1"), Some(&serde_json::json!("SuperSaw")));
        assert!(mgr.can_undo());
        assert!(!mgr.can_redo());

        assert!(mgr.undo_headless(&mut param_map, &mut extra, &shared_synth));
        assert_eq!(extra.preset_name, "Init");
        assert_eq!(param_map.get("o_wav_1"), Some(&serde_json::json!("Saw")));
        assert!(!mgr.can_undo());
        assert!(mgr.can_redo());

        assert!(mgr.redo_headless(&mut param_map, &mut extra, &shared_synth));
        assert_eq!(extra.preset_name, "LD Crimson Hyper");
        assert_eq!(param_map.get("o_wav_1"), Some(&serde_json::json!("SuperSaw")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
