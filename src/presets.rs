use crate::params::{ExtraSynthState, PhantomSpireParams};
use crate::SharedSynthState;
use nih_plug::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PRESET_SCHEMA_VERSION: u32 = 1;
const MAX_UNDO_HISTORY: usize = 64;

const FACTORY_PRESETS_JSON: &[(&str, &str)] = &[
    (
        "01_Init.json",
        include_str!("../presets/factory/01_Init.json"),
    ),
    (
        "02_Phantom_Reese_Bass.json",
        include_str!("../presets/factory/02_Phantom_Reese_Bass.json"),
    ),
    (
        "03_Crimson_Hyper_Lead.json",
        include_str!("../presets/factory/03_Crimson_Hyper_Lead.json"),
    ),
    (
        "04_Velvet_Nebula_Pad.json",
        include_str!("../presets/factory/04_Velvet_Nebula_Pad.json"),
    ),
    (
        "05_Crystal_Glass_Pluck.json",
        include_str!("../presets/factory/05_Crystal_Glass_Pluck.json"),
    ),
    (
        "06_Metaverse_Cyber_Arp.json",
        include_str!("../presets/factory/06_Metaverse_Cyber_Arp.json"),
    ),
    (
        "07_Shibuya_303_Acid.json",
        include_str!("../presets/factory/07_Shibuya_303_Acid.json"),
    ),
    (
        "08_Tokyo_Rave_Chord.json",
        include_str!("../presets/factory/08_Tokyo_Rave_Chord.json"),
    ),
    (
        "09_Shadow_Warp_FX.json",
        include_str!("../presets/factory/09_Shadow_Warp_FX.json"),
    ),
];

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
    pub category: String,
    pub is_factory: bool,
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
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        };
        mgr.rescan();
        mgr
    }

    pub fn user_dir(&self) -> &PathBuf {
        &self.user_dir
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
                    let category = if data.category.trim().is_empty() {
                        String::from("Factory")
                    } else {
                        data.category.clone()
                    };
                    entries.push(PresetEntry {
                        name,
                        category,
                        is_factory: true,
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
                category: String::from("Init"),
                is_factory: true,
                path: None,
                data: PresetData::default(),
            });
        }

        if self.user_dir.is_dir() {
            if let Ok(read_dir) = std::fs::read_dir(&self.user_dir) {
                let mut user_paths: Vec<PathBuf> = read_dir
                    .filter_map(|e| e.ok().map(|entry| entry.path()))
                    .filter(|p| {
                        p.is_file()
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
                        String::from("User")
                    } else {
                        data.category.clone()
                    };
                    entries.push(PresetEntry {
                        name,
                        category,
                        is_factory: false,
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

    pub fn select_next(&mut self) -> usize {
        if !self.entries.is_empty() {
            self.current_index = (self.current_index + 1) % self.entries.len();
        }
        self.current_index
    }

    pub fn select_prev(&mut self) -> usize {
        if !self.entries.is_empty() {
            self.current_index = if self.current_index == 0 {
                self.entries.len() - 1
            } else {
                self.current_index - 1
            };
        }
        self.current_index
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
            data.category = category.trim().to_string();
        } else if data.category.trim().is_empty() {
            data.category = String::from("User");
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
    use std::collections::HashMap;

    fn temp_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phantom_spire_preset_test_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn all_nine_factory_presets_parse_and_match_param_map_keys() {
        assert_eq!(FACTORY_PRESETS_JSON.len(), 9);
        let params = PhantomSpireParams::default();
        let param_map: HashMap<String, ParamPtr> = params
            .param_map()
            .into_iter()
            .map(|(id, ptr, _)| (id, ptr))
            .collect();

        for (filename, raw) in FACTORY_PRESETS_JSON {
            let preset: PresetData = serde_json::from_str(raw)
                .unwrap_or_else(|e| panic!("Factory preset {filename} failed to parse: {e}"));
            assert_eq!(preset.schema_version, PRESET_SCHEMA_VERSION);
            assert!(!preset.name.trim().is_empty(), "{filename} empty name");
            assert!(!preset.category.trim().is_empty(), "{filename} empty category");

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
        }
    }

    #[test]
    fn user_preset_save_load_delete_and_wrap_navigation() {
        let dir = temp_test_dir("crud");
        let mut mgr = PresetManager::with_user_dir(dir.clone());
        assert_eq!(mgr.entries().len(), 9);
        assert_eq!(mgr.current_index(), 0);
        assert_eq!(mgr.current_preset().name, "Init");

        // Wrap prev from 0 -> 8
        assert_eq!(mgr.select_prev(), 8);
        // Wrap next from 8 -> 0
        assert_eq!(mgr.select_next(), 0);

        // Factory preset cannot be deleted
        assert!(mgr.delete_user_preset(0).is_err());

        // Save user preset with dirty characters sanitized
        let params = PhantomSpireParams::default();
        let mut data = PresetManager::capture_from_params(&params);
        data.params
            .insert(String::from("f_cut_1"), serde_json::json!(1234.0));
        let saved_path = mgr
            .save_user_preset("My/Cool:Bass?", "Bass", data)
            .expect("save user preset");
        assert!(saved_path.exists());
        assert_eq!(mgr.entries().len(), 10);

        // Reload in fresh manager and verify it's loaded
        let mut mgr2 = PresetManager::with_user_dir(dir.clone());
        assert_eq!(mgr2.entries().len(), 10);
        let user_idx = mgr2
            .entries()
            .iter()
            .position(|e| !e.is_factory)
            .expect("user preset present");
        assert_eq!(
            mgr2.entries()[user_idx].data.params.get("f_cut_1"),
            Some(&serde_json::json!(1234.0))
        );

        // Delete user preset
        mgr2.delete_user_preset(user_idx).expect("delete user preset");
        assert_eq!(mgr2.entries().len(), 9);
        assert!(!saved_path.exists());

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

        // Apply preset 2 ("Crimson Hyper Lead")
        mgr.apply_to_headless(2, &mut param_map, &mut extra, &shared_synth)
            .expect("apply preset 2");
        assert_eq!(extra.preset_name, "Crimson Hyper Lead");
        assert_eq!(param_map.get("o_wav_1"), Some(&serde_json::json!("SuperSaw")));
        assert!(mgr.can_undo());
        assert!(!mgr.can_redo());

        // Undo back to Init
        assert!(mgr.undo_headless(&mut param_map, &mut extra, &shared_synth));
        assert_eq!(extra.preset_name, "Init");
        assert_eq!(param_map.get("o_wav_1"), Some(&serde_json::json!("Saw")));
        assert!(!mgr.can_undo());
        assert!(mgr.can_redo());

        // Redo forward to Crimson Hyper Lead
        assert!(mgr.redo_headless(&mut param_map, &mut extra, &shared_synth));
        assert_eq!(extra.preset_name, "Crimson Hyper Lead");
        assert_eq!(param_map.get("o_wav_1"), Some(&serde_json::json!("SuperSaw")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
