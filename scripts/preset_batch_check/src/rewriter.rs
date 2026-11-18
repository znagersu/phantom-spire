use phantom_spire::presets::PresetData;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentRole {
    Init,
    SubBass,
    AcidBass,
    ReeseBass,
    PunchBass,
    KeysEp,
    KeysOrgan,
    KeysClavPluck,
    StringsPad,
    ChoirPad,
    AmbientPad,
    SupersawLead,
    SoloLead,
    PluckAcoustic,
    PluckSynth,
    SynthBrass,
    SynthPoly,
    SeqArp,
    DrumKick,
    DrumPerc,
    SoundFx,
}

impl InstrumentRole {
    pub fn label(self) -> &'static str {
        match self {
            InstrumentRole::Init => "Init",
            InstrumentRole::SubBass => "Bass/Sub",
            InstrumentRole::AcidBass => "Bass/Acid",
            InstrumentRole::ReeseBass => "Bass/Reese",
            InstrumentRole::PunchBass => "Bass/Punch",
            InstrumentRole::KeysEp => "Keys/EP",
            InstrumentRole::KeysOrgan => "Keys/Organ",
            InstrumentRole::KeysClavPluck => "Keys/Crisp",
            InstrumentRole::StringsPad => "Strings/Pad",
            InstrumentRole::ChoirPad => "Choir/Pad",
            InstrumentRole::AmbientPad => "Warm/Pad",
            InstrumentRole::SupersawLead => "Lead/Stack",
            InstrumentRole::SoloLead => "Lead/Solo",
            InstrumentRole::PluckAcoustic => "Pluck/Org",
            InstrumentRole::PluckSynth => "Pluck/Synth",
            InstrumentRole::SynthBrass => "Synth/Brass",
            InstrumentRole::SynthPoly => "Synth/Poly",
            InstrumentRole::SeqArp => "Seq & Arp",
            InstrumentRole::DrumKick => "Drum/Kick",
            InstrumentRole::DrumPerc => "Drum/Perc",
            InstrumentRole::SoundFx => "FX",
        }
    }
}

fn stable_hash_unit(name: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    ((h % 1000) as f32) / 1000.0
}

fn has_any(hay: &str, keys: &[&str]) -> bool {
    keys.iter().any(|k| hay.contains(k))
}

pub fn classify_role(preset: &PresetData) -> InstrumentRole {
    let cat = preset.category.to_ascii_lowercase();
    let text = format!(
        "{} {}",
        preset.name.to_ascii_lowercase(),
        preset.tags.join(" ").to_ascii_lowercase()
    );

    if cat == "init" {
        return InstrumentRole::Init;
    }
    if cat == "basses" {
        if has_any(&text, &["808", "sub", "sine", "velvet sub", "liquid dnb"]) {
            return InstrumentRole::SubBass;
        }
        if has_any(&text, &["303", "acid", "growl", "vowel", "phased", "neuro", "industrial"]) {
            return InstrumentRole::AcidBass;
        }
        if has_any(&text, &["reese", "wide", "detuned", "unison", "dark matter", "cyberpunk"]) {
            return InstrumentRole::ReeseBass;
        }
        return InstrumentRole::PunchBass;
    }
    if cat == "keys" {
        if has_any(&text, &["organ", "b3", "drawbar", "pipe", "accordion"]) {
            return InstrumentRole::KeysOrgan;
        }
        if has_any(&text, &["harpsi", "clav", "toy", "celesta", "dx7", "m1", "crystal"]) {
            return InstrumentRole::KeysClavPluck;
        }
        return InstrumentRole::KeysEp;
    }
    if cat == "pads" {
        if has_any(&text, &["string", "solina", "adagio", "ribbon", "brass pad"]) {
            return InstrumentRole::StringsPad;
        }
        if has_any(&text, &["choir", "voice", "ghost"]) {
            return InstrumentRole::ChoirPad;
        }
        return InstrumentRole::AmbientPad;
    }
    if cat == "leads" {
        if has_any(
            &text,
            &["hyper", "crimson", "anthem", "supersaw", "big sound", "eurotrancer", "euphoria", "stadium", "rave", "edm"],
        ) {
            return InstrumentRole::SupersawLead;
        }
        return InstrumentRole::SoloLead;
    }
    if cat == "plucks" {
        if has_any(
            &text,
            &["harp", "kalimba", "marimba", "koto", "guitar", "pizzicato", "sitar", "xylophone", "music box", "flute"],
        ) {
            return InstrumentRole::PluckAcoustic;
        }
        return InstrumentRole::PluckSynth;
    }
    if cat == "synth" {
        if has_any(&text, &["brass", "obie", "prophet", "horn", "poly brass"]) {
            return InstrumentRole::SynthBrass;
        }
        if has_any(&text, &["string"]) {
            return InstrumentRole::StringsPad;
        }
        if has_any(&text, &["organ"]) {
            return InstrumentRole::KeysOrgan;
        }
        return InstrumentRole::SynthPoly;
    }
    if cat == "seq & arp" {
        return InstrumentRole::SeqArp;
    }
    if cat == "drums" {
        let nl = preset.name.to_ascii_lowercase();
        if nl.contains("kick") || nl.contains("bass drum") {
            return InstrumentRole::DrumKick;
        }
        return InstrumentRole::DrumPerc;
    }
    InstrumentRole::SoundFx
}

fn insert_common_safety(p: &mut BTreeMap<String, Value>) {
    p.insert("k_lim".into(), json!(true));
    p.insert("k_vol".into(), json!(0.708)); // -3.0 dB nominal headroom
    p.insert("k_boos".into(), json!(false));
    p.insert("x_cmut".into(), json!(false));
    p.insert("x_rmut".into(), json!(false));
    p.insert("x_dmut".into(), json!(false));
    p.insert("x_smut".into(), json!(false));
    p.insert("x_pmut".into(), json!(false));
    // Keep Chorus wet low-cut protective so stereo chorus never muddies sub-bass in mono
    p.insert("x_cloc".into(), json!(130.0));
}

pub fn rewrite_preset_from_scratch(existing: &PresetData) -> (PresetData, InstrumentRole) {
    let role = classify_role(existing);
    let mut out = existing.clone();
    out.schema_version = 1;
    out.params.clear();

    let v = stable_hash_unit(&existing.name); // 0.0..1.0 subtle musical variation per preset
    let text = format!(
        "{} {}",
        existing.name.to_ascii_lowercase(),
        existing.tags.join(" ").to_ascii_lowercase()
    );
    let p = &mut out.params;

    if role == InstrumentRole::Init {
        p.insert("g_voc".into(), json!(16));
        p.insert("g_pmod".into(), json!("Poly"));
        p.insert("o_mod_1".into(), json!("Classic"));
        p.insert("o_wav_1".into(), json!("Saw"));
        p.insert("o_ca_1".into(), json!(0.5));
        p.insert("o_lvl_1".into(), json!(0.8));
        p.insert("f_mod_1".into(), json!("Perfecto"));
        p.insert("f_typ_1".into(), json!("LP4"));
        p.insert("f_cut_1".into(), json!(18000.0));
        p.insert("f_res_1".into(), json!(0.0));
        p.insert("e_att_1".into(), json!(0.003));
        p.insert("e_dec_1".into(), json!(0.4));
        p.insert("e_sus_1".into(), json!(0.8));
        p.insert("e_rel_1".into(), json!(0.15));
        p.insert("k_lim".into(), json!(true));
        p.insert("k_vol".into(), json!(0.708));
        return (out, role);
    }

    insert_common_safety(p);

    match role {
        InstrumentRole::SubBass => {
            p.insert("g_voc".into(), json!(1));
            p.insert("g_pmod".into(), json!("Mono 2"));
            p.insert("g_gld".into(), json!(18.0 + v * 16.0));
            p.insert("g_glog".into(), json!(true));
            p.insert("g_drft".into(), json!(true));
            p.insert("k_vol".into(), json!(0.95));

            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!(if text.contains("sine") { "Sin" } else { "Sub" }));
            p.insert("o_oct_1".into(), json!(-1));
            p.insert("o_ca_1".into(), json!(0.45));
            p.insert("o_wid_1".into(), json!(0.0));
            p.insert("o_lvl_1".into(), json!(0.92));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Bass"));
            p.insert("o_oct_2".into(), json!(0));
            p.insert("o_wid_2".into(), json!(0.0));
            p.insert("o_lvl_2".into(), json!(0.34 + v * 0.08));

            p.insert("f_mod_1".into(), json!("Infecto"));
            p.insert("f_typ_1".into(), json!("LP4"));
            p.insert("f_cut_1".into(), json!(1100.0 + v * 450.0));
            p.insert("f_res_1".into(), json!(0.10));
            p.insert("f_drv_1".into(), json!(0.14));

            p.insert("e_att_1".into(), json!(0.003));
            p.insert("e_dec_1".into(), json!(0.42 + v * 0.18));
            p.insert("e_sus_1".into(), json!(if text.contains("808") { 0.62 } else { 0.78 }));
            p.insert("e_rel_1".into(), json!(0.12));

            p.insert("e_att_2".into(), json!(0.002));
            p.insert("e_dec_2".into(), json!(0.14));
            p.insert("e_sus_2".into(), json!(0.0));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.24));

            p.insert("x_smod".into(), json!("Warm"));
            p.insert("x_sdrv".into(), json!(0.26));
            p.insert("x_sdw".into(), json!(0.14));
            p.insert("x_cdw".into(), json!(0.0));
            p.insert("x_ddw".into(), json!(0.0));
            p.insert("x_rdw".into(), json!(0.0));
            p.insert("k_warm".into(), json!(true));
        }

        InstrumentRole::AcidBass => {
            p.insert("g_voc".into(), json!(1));
            p.insert("g_pmod".into(), json!("Mono 2"));
            p.insert("g_gld".into(), json!(24.0 + v * 14.0));
            p.insert("g_glog".into(), json!(true));
            p.insert("g_drft".into(), json!(true));
            p.insert("k_vol".into(), json!(0.88));

            let wav1 = if text.contains("vowel") {
                "Formant"
            } else if text.contains("acid") || text.contains("303") {
                "Acid"
            } else {
                "Saw"
            };
            p.insert("o_mod_1".into(), json!(if text.contains("vowel") { "Vowel" } else { "Classic" }));
            p.insert("o_wav_1".into(), json!(wav1));
            p.insert("o_oct_1".into(), json!(-1));
            p.insert("o_ca_1".into(), json!(0.50 + v * 0.2));
            p.insert("o_wid_1".into(), json!(0.20));
            p.insert("o_lvl_1".into(), json!(0.86));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Saw"));
            p.insert("o_oct_2".into(), json!(0));
            p.insert("o_fin_2".into(), json!(-3.0 + v * 6.0));
            p.insert("o_lvl_2".into(), json!(0.38));

            p.insert("f_mod_1".into(), json!(if text.contains("acid") || text.contains("303") { "Acido" } else { "Scorpio" }));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(2150.0 + v * 750.0));
            p.insert("f_res_1".into(), json!(0.34 + v * 0.14));
            p.insert("f_drv_1".into(), json!(0.22));

            p.insert("e_att_1".into(), json!(0.003));
            p.insert("e_dec_1".into(), json!(0.38 + v * 0.14));
            p.insert("e_sus_1".into(), json!(0.55));
            p.insert("e_rel_1".into(), json!(0.10));

            p.insert("e_att_2".into(), json!(0.002));
            p.insert("e_dec_2".into(), json!(0.20 + v * 0.08));
            p.insert("e_sus_2".into(), json!(0.05));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.42));
            p.insert("es_vel_1_2".into(), json!(0.35));

            p.insert("x_smod".into(), json!("Tube"));
            p.insert("x_sdrv".into(), json!(0.34));
            p.insert("x_sdw".into(), json!(0.18));
            if text.contains("phased") {
                p.insert("x_pdw".into(), json!(0.22));
            }
            p.insert("x_cdw".into(), json!(0.0));
            p.insert("x_ddw".into(), json!(0.0));
            p.insert("x_rdw".into(), json!(0.06));
        }

        InstrumentRole::ReeseBass => {
            p.insert("g_voc".into(), json!(8));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_gld".into(), json!(16.0));
            p.insert("g_glog".into(), json!(true));
            p.insert("g_drft".into(), json!(true));

            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!("SuperSaw"));
            p.insert("o_oct_1".into(), json!(-1));
            p.insert("o_ucnt_1".into(), json!(5));
            p.insert("o_udet_1".into(), json!(0.26 + v * 0.08));
            p.insert("o_udns_1".into(), json!(0.65));
            p.insert("o_wid_1".into(), json!(0.58));
            p.insert("o_lvl_1".into(), json!(0.66));

            // Focused mono center sub foundation
            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Sub"));
            p.insert("o_oct_2".into(), json!(-1));
            p.insert("o_ucnt_2".into(), json!(1));
            p.insert("o_wid_2".into(), json!(0.0));
            p.insert("o_lvl_2".into(), json!(0.46));

            p.insert("f_mod_1".into(), json!("Infecto"));
            p.insert("f_typ_1".into(), json!("LP4"));
            p.insert("f_cut_1".into(), json!(1600.0 + v * 700.0));
            p.insert("f_res_1".into(), json!(0.18));
            p.insert("f_drv_1".into(), json!(0.16));

            p.insert("e_att_1".into(), json!(0.004));
            p.insert("e_dec_1".into(), json!(0.45));
            p.insert("e_sus_1".into(), json!(0.78));
            p.insert("e_rel_1".into(), json!(0.16));

            p.insert("e_att_2".into(), json!(0.003));
            p.insert("e_dec_2".into(), json!(0.24));
            p.insert("e_sus_2".into(), json!(0.10));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.24));

            p.insert("l_rat_1".into(), json!(0.8 + v * 0.7));
            p.insert("ls_tgt_1_1".into(), json!("Filter1 Cutoff"));
            p.insert("ls_amt_1_1".into(), json!(0.12));

            p.insert("x_smod".into(), json!("Warm"));
            p.insert("x_sdrv".into(), json!(0.24));
            p.insert("x_sdw".into(), json!(0.14));
            p.insert("x_cmod".into(), json!("Chorus"));
            p.insert("x_cdw".into(), json!(0.14));
            p.insert("x_rmod".into(), json!("Room"));
            p.insert("x_rdw".into(), json!(0.08));
        }

        InstrumentRole::PunchBass => {
            p.insert("g_voc".into(), json!(8));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            let (mode1, wav1) = if text.contains("fm") || text.contains("donk") || text.contains("slap") {
                ("FM", "Bell")
            } else if text.contains("organ") {
                ("Classic", "Organ")
            } else if text.contains("hollow") || text.contains("wood") {
                ("Classic", "Hollow")
            } else {
                ("Classic", "Bass")
            };
            p.insert("o_mod_1".into(), json!(mode1));
            p.insert("o_wav_1".into(), json!(wav1));
            p.insert("o_oct_1".into(), json!(-1));
            p.insert("o_ca_1".into(), json!(if mode1 == "FM" { 0.34 } else { 0.55 }));
            p.insert("o_cb_1".into(), json!(if mode1 == "FM" { 0.38 } else { 0.12 }));
            p.insert("o_ucnt_1".into(), json!(2));
            p.insert("o_udet_1".into(), json!(0.12));
            p.insert("o_wid_1".into(), json!(0.35));
            p.insert("o_lvl_1".into(), json!(0.56));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Saw"));
            p.insert("o_oct_2".into(), json!(0));
            p.insert("o_fin_2".into(), json!(-3.0 + v * 6.0));
            p.insert("o_lvl_2".into(), json!(0.54));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(2850.0 + v * 950.0));
            p.insert("f_res_1".into(), json!(0.20));
            p.insert("f_drv_1".into(), json!(0.22));

            p.insert("e_att_1".into(), json!(0.003));
            p.insert("e_dec_1".into(), json!(0.34 + v * 0.14));
            p.insert("e_sus_1".into(), json!(0.42 + v * 0.18));
            p.insert("e_rel_1".into(), json!(0.12));

            p.insert("e_att_2".into(), json!(0.002));
            p.insert("e_dec_2".into(), json!(0.15 + v * 0.06));
            p.insert("e_sus_2".into(), json!(0.04));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.38));

            p.insert("x_smod".into(), json!("Soft"));
            p.insert("x_sdrv".into(), json!(0.30));
            p.insert("x_sdw".into(), json!(0.22));
            p.insert("x_cdw".into(), json!(0.10));
            p.insert("x_rmod".into(), json!("Room"));
            p.insert("x_rdw".into(), json!(0.08));
        }

        InstrumentRole::KeysEp => {
            p.insert("g_voc".into(), json!(12));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));
            p.insert("k_vel".into(), json!(0.65));

            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!(if text.contains("wurli") { "Hollow" } else { "Tri" }));
            p.insert("o_ca_1".into(), json!(0.45));
            p.insert("o_ucnt_1".into(), json!(2));
            p.insert("o_udet_1".into(), json!(0.10));
            p.insert("o_wid_1".into(), json!(0.42));
            p.insert("o_lvl_1".into(), json!(0.72));

            // Bell tine attack harmonics via FM osc
            p.insert("o_mod_2".into(), json!("FM"));
            p.insert("o_wav_2".into(), json!("Sin"));
            p.insert("o_oct_2".into(), json!(1));
            p.insert("o_ca_2".into(), json!(0.28 + v * 0.14));
            p.insert("o_cb_2".into(), json!(0.26));
            p.insert("o_lvl_2".into(), json!(0.30));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(4400.0 + v * 2200.0));
            p.insert("f_res_1".into(), json!(0.08));
            p.insert("f_ktr_1".into(), json!(0.25));

            p.insert("e_att_1".into(), json!(0.004));
            p.insert("e_dec_1".into(), json!(0.95 + v * 0.35));
            p.insert("e_sus_1".into(), json!(0.40 + v * 0.10));
            p.insert("e_rel_1".into(), json!(0.28));

            p.insert("e_att_2".into(), json!(0.002));
            p.insert("e_dec_2".into(), json!(0.18));
            p.insert("e_sus_2".into(), json!(0.12));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.18));

            p.insert("x_cmod".into(), json!("Chorus"));
            p.insert("x_crat".into(), json!(0.45));
            p.insert("x_cdpt".into(), json!(0.42));
            p.insert("x_cdw".into(), json!(0.20));
            p.insert("x_rmod".into(), json!("Plate 1"));
            p.insert("x_rdec".into(), json!(1.6));
            p.insert("x_rdw".into(), json!(0.18));
            p.insert("k_warm".into(), json!(true));
        }

        InstrumentRole::KeysOrgan => {
            p.insert("g_voc".into(), json!(14));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!("Organ"));
            p.insert("o_ucnt_1".into(), json!(2));
            p.insert("o_udet_1".into(), json!(0.12));
            p.insert("o_wid_1".into(), json!(0.48));
            p.insert("o_lvl_1".into(), json!(0.72));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!(if text.contains("pipe") { "Choir" } else { "Hollow" }));
            p.insert("o_oct_2".into(), json!(1));
            p.insert("o_fin_2".into(), json!(3.5));
            p.insert("o_lvl_2".into(), json!(0.34));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(5600.0 + v * 2800.0));
            p.insert("f_res_1".into(), json!(0.08));

            // Realistic fast organ ASR envelope
            p.insert("e_att_1".into(), json!(0.006));
            p.insert("e_dec_1".into(), json!(0.25));
            p.insert("e_sus_1".into(), json!(0.90));
            p.insert("e_rel_1".into(), json!(if text.contains("pipe") { 0.24 } else { 0.12 }));

            p.insert("x_cmod".into(), json!("Chorus"));
            p.insert("x_crat".into(), json!(2.4 + v * 1.2));
            p.insert("x_cdw".into(), json!(0.20));
            p.insert("x_rmod".into(), json!(if text.contains("cathedral") || text.contains("pipe") { "Hall" } else { "Room" }));
            p.insert("x_rdec".into(), json!(if text.contains("cathedral") { 2.8 } else { 1.4 }));
            p.insert("x_rdw".into(), json!(0.18));
        }

        InstrumentRole::KeysClavPluck => {
            p.insert("g_voc".into(), json!(12));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("k_vel".into(), json!(0.70));

            let is_bell = has_any(&text, &["celesta", "toy", "dx7", "crystal"]);
            p.insert("o_mod_1".into(), json!(if is_bell { "FM" } else { "Classic" }));
            p.insert("o_wav_1".into(), json!(if is_bell { "Bell" } else if text.contains("clav") { "Pulse" } else { "Saw" }));
            p.insert("o_ca_1".into(), json!(if is_bell { 0.42 } else { 0.35 }));
            p.insert("o_cb_1".into(), json!(if is_bell { 0.36 } else { 0.20 }));
            p.insert("o_ucnt_1".into(), json!(2));
            p.insert("o_udet_1".into(), json!(0.12));
            p.insert("o_wid_1".into(), json!(0.50));
            p.insert("o_lvl_1".into(), json!(0.72));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Tri"));
            p.insert("o_oct_2".into(), json!(if is_bell { 1 } else { 0 }));
            p.insert("o_lvl_2".into(), json!(0.28));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(5400.0 + v * 3600.0));
            p.insert("f_res_1".into(), json!(0.12));
            p.insert("f_ktr_1".into(), json!(0.30));

            p.insert("e_att_1".into(), json!(0.003));
            p.insert("e_dec_1".into(), json!(0.55 + v * 0.25));
            p.insert("e_sus_1".into(), json!(0.32));
            p.insert("e_rel_1".into(), json!(0.24));

            p.insert("x_cmod".into(), json!("Chorus"));
            p.insert("x_cdw".into(), json!(0.14));
            p.insert("x_dsyn".into(), json!(true));
            p.insert("x_ddw".into(), json!(0.14));
            p.insert("x_rmod".into(), json!("Plate 1"));
            p.insert("x_rdw".into(), json!(0.18));
        }

        InstrumentRole::StringsPad => {
            p.insert("g_voc".into(), json!(12));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!("Strings"));
            p.insert("o_ucnt_1".into(), json!(5));
            p.insert("o_udet_1".into(), json!(0.24 + v * 0.08));
            p.insert("o_udns_1".into(), json!(0.65));
            p.insert("o_wid_1".into(), json!(0.74));
            p.insert("o_lvl_1".into(), json!(0.64));

            p.insert("o_mod_2".into(), json!("SawPWM"));
            p.insert("o_wav_2".into(), json!("Saw"));
            p.insert("o_oct_2".into(), json!(1));
            p.insert("o_ca_2".into(), json!(0.48));
            p.insert("o_ucnt_2".into(), json!(3));
            p.insert("o_udet_2".into(), json!(0.20));
            p.insert("o_wid_2".into(), json!(0.68));
            p.insert("o_lvl_2".into(), json!(0.28));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(3200.0 + v * 1600.0));
            p.insert("f_res_1".into(), json!(0.10));
            p.insert("f_ktr_1".into(), json!(0.25));

            // Realistic bowed string swell that opens fully inside chord hold
            p.insert("e_att_1".into(), json!(0.14 + v * 0.08));
            p.insert("e_dec_1".into(), json!(0.70));
            p.insert("e_sus_1".into(), json!(0.86));
            p.insert("e_rel_1".into(), json!(0.48));

            p.insert("e_att_2".into(), json!(0.18));
            p.insert("e_dec_2".into(), json!(0.65));
            p.insert("e_sus_2".into(), json!(0.45));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.20));

            p.insert("l_rat_1".into(), json!(4.6));
            p.insert("l_fad_1".into(), json!(0.35));
            p.insert("ls_tgt_1_1".into(), json!("All Pitch"));
            p.insert("ls_amt_1_1".into(), json!(0.025));

            p.insert("x_cmod".into(), json!("Ensemble"));
            p.insert("x_cdw".into(), json!(0.24));
            p.insert("x_rmod".into(), json!("Hall"));
            p.insert("x_rdec".into(), json!(2.6));
            p.insert("x_rdmp".into(), json!(0.45));
            p.insert("x_rdw".into(), json!(0.24));
        }

        InstrumentRole::ChoirPad => {
            p.insert("g_voc".into(), json!(12));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!("Choir"));
            p.insert("o_ucnt_1".into(), json!(5));
            p.insert("o_udet_1".into(), json!(0.22));
            p.insert("o_wid_1".into(), json!(0.72));
            p.insert("o_lvl_1".into(), json!(0.64));

            p.insert("o_mod_2".into(), json!("Vowel"));
            p.insert("o_wav_2".into(), json!("Formant"));
            p.insert("o_ca_2".into(), json!(0.30 + v * 0.40));
            p.insert("o_cb_2".into(), json!(0.42));
            p.insert("o_lvl_2".into(), json!(0.30));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(3400.0 + v * 1400.0));
            p.insert("f_res_1".into(), json!(0.12));

            p.insert("e_att_1".into(), json!(0.16 + v * 0.08));
            p.insert("e_dec_1".into(), json!(0.75));
            p.insert("e_sus_1".into(), json!(0.86));
            p.insert("e_rel_1".into(), json!(0.50));

            p.insert("l_rat_1".into(), json!(0.45));
            p.insert("ls_tgt_1_1".into(), json!("OSC2 CtrlA"));
            p.insert("ls_amt_1_1".into(), json!(0.12));

            p.insert("x_cmod".into(), json!("Ensemble"));
            p.insert("x_cdw".into(), json!(0.22));
            p.insert("x_rmod".into(), json!("Hall"));
            p.insert("x_rdec".into(), json!(2.8));
            p.insert("x_rdmp".into(), json!(0.45));
            p.insert("x_rdw".into(), json!(0.26));
        }

        InstrumentRole::AmbientPad => {
            p.insert("g_voc".into(), json!(12));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            let is_fifth = text.contains("5th") || text.contains("fifth");
            let is_shimmer = has_any(&text, &["shimmer", "crystal", "glass", "aurora", "starlight"]);
            p.insert("o_mod_1".into(), json!("SawPWM"));
            p.insert("o_wav_1".into(), json!(if is_shimmer { "Bell" } else { "Saw" }));
            p.insert("o_ca_1".into(), json!(0.45 + v * 0.25));
            p.insert("o_cb_1".into(), json!(0.30));
            p.insert("o_ucnt_1".into(), json!(5));
            p.insert("o_udet_1".into(), json!(0.24 + v * 0.08));
            p.insert("o_wid_1".into(), json!(0.76));
            p.insert("o_lvl_1".into(), json!(0.62));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Tri"));
            p.insert("o_oct_2".into(), json!(if is_shimmer { 1 } else { 0 }));
            if is_fifth {
                p.insert("o_not_2".into(), json!(7));
            }
            p.insert("o_lvl_2".into(), json!(0.30));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(2500.0 + v * 2000.0));
            p.insert("f_res_1".into(), json!(0.14));
            p.insert("f_ktr_1".into(), json!(0.20));

            p.insert("e_att_1".into(), json!(0.11 + v * 0.08));
            p.insert("e_dec_1".into(), json!(0.75));
            p.insert("e_sus_1".into(), json!(0.84));
            p.insert("e_rel_1".into(), json!(0.48 + v * 0.16));

            p.insert("e_att_2".into(), json!(0.16));
            p.insert("e_dec_2".into(), json!(0.75));
            p.insert("e_sus_2".into(), json!(0.45));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.22));

            p.insert("l_rat_1".into(), json!(0.32 + v * 0.25));
            p.insert("ls_tgt_1_1".into(), json!("Filter1 Cutoff"));
            p.insert("ls_amt_1_1".into(), json!(0.14));

            p.insert("x_cmod".into(), json!("Dimension"));
            p.insert("x_cdw".into(), json!(0.22));
            p.insert("x_dsyn".into(), json!(true));
            p.insert("x_ddw".into(), json!(0.14));
            p.insert("x_rmod".into(), json!("Plate 2"));
            p.insert("x_rdec".into(), json!(2.8));
            p.insert("x_rdmp".into(), json!(0.48));
            p.insert("x_rdw".into(), json!(0.26));
        }

        InstrumentRole::SupersawLead => {
            p.insert("g_voc".into(), json!(12));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            // Preserves SuperSaw on OSC1 for LD Crimson Hyper & anthem stacks
            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!("SuperSaw"));
            p.insert("o_ca_1".into(), json!(0.54));
            p.insert("o_ucnt_1".into(), json!(7));
            p.insert("o_udet_1".into(), json!(0.30 + v * 0.08));
            p.insert("o_udns_1".into(), json!(0.65));
            p.insert("o_wid_1".into(), json!(0.76));
            p.insert("o_lvl_1".into(), json!(0.68));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Square"));
            p.insert("o_oct_2".into(), json!(1));
            p.insert("o_ucnt_2".into(), json!(3));
            p.insert("o_udet_2".into(), json!(0.18));
            p.insert("o_wid_2".into(), json!(0.60));
            p.insert("o_lvl_2".into(), json!(0.30));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP4"));
            p.insert("f_cut_1".into(), json!(6500.0 + v * 4500.0));
            p.insert("f_res_1".into(), json!(0.14));
            p.insert("f_drv_1".into(), json!(0.08));

            p.insert("e_att_1".into(), json!(0.004));
            p.insert("e_dec_1".into(), json!(0.40));
            p.insert("e_sus_1".into(), json!(0.84));
            p.insert("e_rel_1".into(), json!(0.20));

            p.insert("l_rat_1".into(), json!(5.2));
            p.insert("l_fad_1".into(), json!(0.30));
            p.insert("ls_tgt_1_1".into(), json!("All Pitch"));
            p.insert("ls_amt_1_1".into(), json!(0.035));

            p.insert("x_smod".into(), json!("Tube"));
            p.insert("x_sdrv".into(), json!(0.26));
            p.insert("x_sdw".into(), json!(0.12));
            p.insert("x_dsyn".into(), json!(true));
            p.insert("x_dfb".into(), json!(0.32));
            p.insert("x_ddw".into(), json!(0.18));
            p.insert("x_rmod".into(), json!("Hall"));
            p.insert("x_rdec".into(), json!(2.2));
            p.insert("x_rdw".into(), json!(0.18));
        }

        InstrumentRole::SoloLead => {
            let is_legato = has_any(&text, &["glide", "solo", "flute", "velvet"]);
            let is_sync = text.contains("sync");
            let is_acid = has_any(&text, &["angry", "reso", "acid"]);
            let is_chip = has_any(&text, &["8_bit", "8 bit", "chip", "retro"]);

            p.insert("g_voc".into(), json!(if is_legato { 4 } else { 10 }));
            p.insert("g_pmod".into(), json!(if is_legato { "Legato" } else { "Poly" }));
            p.insert("g_gld".into(), json!(if is_legato { 55.0 } else { 10.0 }));
            p.insert("g_glog".into(), json!(true));
            p.insert("g_drft".into(), json!(true));

            p.insert("o_mod_1".into(), json!(if is_sync { "AMSync" } else { "Classic" }));
            p.insert("o_wav_1".into(), json!(if is_chip { "Square" } else { "Saw" }));
            p.insert("o_ca_1".into(), json!(if is_sync { 0.24 } else { 0.52 }));
            p.insert("o_cb_1".into(), json!(if is_sync { 0.14 } else { 0.15 }));
            p.insert("o_ucnt_1".into(), json!(if is_chip { 1 } else { 3 }));
            p.insert("o_udet_1".into(), json!(0.20));
            p.insert("o_wid_1".into(), json!(0.58));
            p.insert("o_lvl_1".into(), json!(if is_sync { 0.64 } else { 0.72 }));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Saw"));
            p.insert("o_oct_2".into(), json!(if is_sync || text.contains("electro") { -1 } else if v > 0.65 { 1 } else { 0 }));
            p.insert("o_fin_2".into(), json!(5.0));
            p.insert("o_lvl_2".into(), json!(if is_sync { 0.44 } else { 0.34 }));

            p.insert("f_mod_1".into(), json!(if is_acid { "Acido" } else { "Perfecto" }));
            p.insert("f_typ_1".into(), json!("LP4"));
            p.insert("f_cut_1".into(), json!(if is_sync || text.contains("electro") { 2400.0 + v * 700.0 } else { 4200.0 + v * 3200.0 }));
            p.insert("f_res_1".into(), json!(if is_acid { 0.34 } else { 0.16 }));
            p.insert("f_ktr_1".into(), json!(if is_sync { 0.18 } else { 0.35 }));

            p.insert("e_att_1".into(), json!(0.004));
            p.insert("e_dec_1".into(), json!(0.42));
            p.insert("e_sus_1".into(), json!(0.82));
            p.insert("e_rel_1".into(), json!(0.18));

            p.insert("e_att_2".into(), json!(0.003));
            p.insert("e_dec_2".into(), json!(0.22));
            p.insert("e_sus_2".into(), json!(0.12));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.22));

            p.insert("l_rat_1".into(), json!(5.0));
            p.insert("l_fad_1".into(), json!(0.32));
            p.insert("ls_tgt_1_1".into(), json!("All Pitch"));
            p.insert("ls_amt_1_1".into(), json!(0.04));

            p.insert("x_smod".into(), json!(if is_chip { "Bitcrush" } else { "Warm" }));
            p.insert("x_sdrv".into(), json!(if is_chip { 0.30 } else { 0.25 }));
            p.insert("x_sdw".into(), json!(0.14));
            p.insert("x_dsyn".into(), json!(true));
            p.insert("x_dfb".into(), json!(0.34));
            p.insert("x_ddw".into(), json!(0.18));
            p.insert("x_rmod".into(), json!("Plate 1"));
            p.insert("x_rdw".into(), json!(0.18));
        }

        InstrumentRole::PluckAcoustic | InstrumentRole::PluckSynth => {
            p.insert("g_voc".into(), json!(12));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("k_vel".into(), json!(0.65));

            let is_org = role == InstrumentRole::PluckAcoustic;
            p.insert("o_mod_1".into(), json!(if is_org && v > 0.55 { "FM" } else { "Classic" }));
            p.insert("o_wav_1".into(), json!(if is_org { "Hollow" } else { "Saw" }));
            p.insert("o_ca_1".into(), json!(0.44 + v * 0.2));
            p.insert("o_cb_1".into(), json!(0.24));
            p.insert("o_ucnt_1".into(), json!(if is_org { 2 } else { 5 }));
            p.insert("o_udet_1".into(), json!(if is_org { 0.12 } else { 0.24 }));
            p.insert("o_wid_1".into(), json!(0.62));
            p.insert("o_lvl_1".into(), json!(0.74));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!(if is_org { "Tri" } else { "Square" }));
            p.insert("o_oct_2".into(), json!(if text.contains("sub") { -1 } else { 0 }));
            p.insert("o_lvl_2".into(), json!(0.32));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP4"));
            // Open enough baseline + musical pluck filter envelope so notes stay articulated, never muffled or thin
            p.insert("f_cut_1".into(), json!(2400.0 + v * 1200.0));
            p.insert("f_res_1".into(), json!(0.14));
            p.insert("f_ktr_1".into(), json!(0.35));

            p.insert("e_att_1".into(), json!(0.003));
            p.insert("e_dec_1".into(), json!(0.44 + v * 0.16));
            p.insert("e_sus_1".into(), json!(0.24 + v * 0.08));
            p.insert("e_rel_1".into(), json!(0.20 + v * 0.08));

            p.insert("e_att_2".into(), json!(0.002));
            p.insert("e_dec_2".into(), json!(0.20 + v * 0.08));
            p.insert("e_sus_2".into(), json!(0.08));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.38));
            p.insert("es_vel_1_2".into(), json!(0.35));

            p.insert("x_cmod".into(), json!("Chorus"));
            p.insert("x_cdw".into(), json!(0.14));
            p.insert("x_dsyn".into(), json!(true));
            p.insert("x_dpp".into(), json!(true));
            p.insert("x_dfb".into(), json!(0.34));
            p.insert("x_ddw".into(), json!(0.18));
            p.insert("x_rmod".into(), json!("Plate 1"));
            p.insert("x_rdec".into(), json!(1.9));
            p.insert("x_rdw".into(), json!(0.18));
        }

        InstrumentRole::SynthBrass | InstrumentRole::SynthPoly => {
            p.insert("g_voc".into(), json!(12));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            let is_brass = role == InstrumentRole::SynthBrass;
            let is_bell_synth = has_any(&text, &["bell", "glassy", "metallic"]);
            let is_vocal_synth = text.contains("vocal");
            p.insert("o_mod_1".into(), json!(if is_bell_synth { "FM" } else if is_vocal_synth { "Vowel" } else { "Classic" }));
            p.insert("o_wav_1".into(), json!(if is_bell_synth { "Bell" } else if is_vocal_synth { "Choir" } else { "Saw" }));
            p.insert("o_ca_1".into(), json!(if is_bell_synth { 0.42 } else { 0.50 }));
            p.insert("o_cb_1".into(), json!(if is_bell_synth { 0.34 } else { 0.18 }));
            p.insert("o_ucnt_1".into(), json!(if is_brass { 4 } else { 5 }));
            p.insert("o_udet_1".into(), json!(0.22 + v * 0.06));
            p.insert("o_wid_1".into(), json!(0.68));
            p.insert("o_lvl_1".into(), json!(0.68));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!(if is_brass { "Saw" } else { "Square" }));
            p.insert("o_fin_2".into(), json!(6.0));
            p.insert("o_lvl_2".into(), json!(0.32));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP4"));
            p.insert("f_cut_1".into(), json!(if is_brass { 2800.0 + v * 1400.0 } else if is_bell_synth { 6400.0 + v * 3200.0 } else { 4000.0 + v * 2800.0 }));
            p.insert("f_res_1".into(), json!(0.16));
            p.insert("f_ktr_1".into(), json!(0.25));

            p.insert("e_att_1".into(), json!(if is_brass { 0.040 + v * 0.02 } else { 0.004 }));
            p.insert("e_dec_1".into(), json!(0.48));
            p.insert("e_sus_1".into(), json!(if is_brass { 0.78 } else { 0.68 }));
            p.insert("e_rel_1".into(), json!(0.24));

            p.insert("e_att_2".into(), json!(if is_brass { 0.055 } else { 0.003 }));
            p.insert("e_dec_2".into(), json!(0.35));
            p.insert("e_sus_2".into(), json!(0.25));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(if is_brass { 0.36 } else { 0.24 }));

            p.insert("x_cmod".into(), json!("Chorus"));
            p.insert("x_cdw".into(), json!(0.20));
            p.insert("x_dsyn".into(), json!(true));
            p.insert("x_ddw".into(), json!(0.14));
            p.insert("x_rmod".into(), json!("Plate 1"));
            p.insert("x_rdec".into(), json!(2.0));
            p.insert("x_rdw".into(), json!(0.18));
        }

        InstrumentRole::SeqArp => {
            p.insert("g_voc".into(), json!(10));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            let is_acid_seq = text.contains("acid") || text.contains("303");
            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!(if is_acid_seq { "Acid" } else { "Saw" }));
            p.insert("o_ucnt_1".into(), json!(if is_acid_seq { 2 } else { 3 }));
            p.insert("o_udet_1".into(), json!(0.20));
            p.insert("o_wid_1".into(), json!(0.58));
            p.insert("o_lvl_1".into(), json!(0.72));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Square"));
            p.insert("o_oct_2".into(), json!(if text.contains("bass") { -1 } else { 0 }));
            p.insert("o_lvl_2".into(), json!(0.30));

            p.insert("f_mod_1".into(), json!(if is_acid_seq { "Acido" } else { "Perfecto" }));
            p.insert("f_typ_1".into(), json!("LP4"));
            p.insert("f_cut_1".into(), json!(2800.0 + v * 2200.0));
            p.insert("f_res_1".into(), json!(if is_acid_seq { 0.34 } else { 0.18 }));
            p.insert("k_vol".into(), json!(if is_acid_seq { 0.85 } else { 0.72 }));

            p.insert("e_att_1".into(), json!(0.003));
            p.insert("e_dec_1".into(), json!(0.28 + v * 0.12));
            p.insert("e_sus_1".into(), json!(0.36));
            p.insert("e_rel_1".into(), json!(0.14));

            p.insert("e_att_2".into(), json!(0.002));
            p.insert("e_dec_2".into(), json!(0.18));
            p.insert("e_sus_2".into(), json!(0.10));
            p.insert("es_tgt_1_2".into(), json!("Filter1 Cutoff"));
            p.insert("es_amt_1_2".into(), json!(0.36));

            p.insert("a_on".into(), json!(true));
            p.insert("a_mod".into(), json!(if v > 0.65 { "Up/Down" } else { "Up" }));
            p.insert("a_oct".into(), json!(2));
            p.insert("a_syn".into(), json!(true));
            p.insert("a_sdiv".into(), json!(if text.contains("8th") { "1/8" } else { "1/16" }));
            p.insert("a_gat".into(), json!(0.75));

            p.insert("x_dsyn".into(), json!(true));
            p.insert("x_dpp".into(), json!(true));
            p.insert("x_dfb".into(), json!(0.34));
            p.insert("x_ddw".into(), json!(0.18));
            p.insert("x_rmod".into(), json!("Plate 1"));
            p.insert("x_rdw".into(), json!(0.16));
        }

        InstrumentRole::DrumKick => {
            p.insert("g_voc".into(), json!(4));
            p.insert("g_pmod".into(), json!("Mono 1"));

            p.insert("o_mod_1".into(), json!("Classic"));
            p.insert("o_wav_1".into(), json!("Sin"));
            p.insert("o_oct_1".into(), json!(-1));
            p.insert("o_lvl_1".into(), json!(0.82));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Tri"));
            p.insert("o_oct_2".into(), json!(0));
            p.insert("o_lvl_2".into(), json!(0.24));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(2200.0));
            p.insert("f_res_1".into(), json!(0.12));

            p.insert("e_att_1".into(), json!(0.003));
            p.insert("e_dec_1".into(), json!(0.38));
            p.insert("e_sus_1".into(), json!(0.18));
            p.insert("e_rel_1".into(), json!(0.12));

            // Pitch thump via ENV2 -> AllPitch
            p.insert("e_att_2".into(), json!(0.001));
            p.insert("e_dec_2".into(), json!(0.055));
            p.insert("e_sus_2".into(), json!(0.0));
            p.insert("es_tgt_1_2".into(), json!("All Pitch"));
            p.insert("es_amt_1_2".into(), json!(0.42));

            p.insert("x_smod".into(), json!("Warm"));
            p.insert("x_sdrv".into(), json!(0.28));
            p.insert("x_sdw".into(), json!(0.16));
            p.insert("x_rdw".into(), json!(0.0));
        }

        InstrumentRole::DrumPerc => {
            p.insert("g_voc".into(), json!(8));
            p.insert("g_pmod".into(), json!("Poly"));

            let is_hat = has_any(&text, &["hat", "crash", "shaker"]);
            let is_snare_clap = has_any(&text, &["snare", "clap", "rim"]);
            p.insert("o_mod_1".into(), json!(if is_hat || is_snare_clap { "Noise" } else { "FM" }));
            p.insert("o_wav_1".into(), json!(if is_hat { "Metallic" } else { "Bell" }));
            p.insert("o_oct_1".into(), json!(if is_hat { 1 } else { 0 }));
            p.insert("o_ca_1".into(), json!(if is_hat { 0.72 } else { 0.48 }));
            p.insert("o_cb_1".into(), json!(0.24));
            p.insert("o_lvl_1".into(), json!(0.68));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Tri"));
            p.insert("o_lvl_2".into(), json!(if is_hat { 0.14 } else { 0.36 }));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!(if is_hat { "HP2" } else { "LP2" }));
            p.insert("f_cut_1".into(), json!(if is_hat { 2400.0 } else { 4200.0 }));
            p.insert("f_res_1".into(), json!(0.18));

            p.insert("e_att_1".into(), json!(0.003));
            p.insert("e_dec_1".into(), json!(if text.contains("open") || text.contains("crash") { 0.45 } else { 0.24 }));
            p.insert("e_sus_1".into(), json!(0.12));
            p.insert("e_rel_1".into(), json!(0.12));

            p.insert("e_att_2".into(), json!(0.001));
            p.insert("e_dec_2".into(), json!(0.07));
            p.insert("es_tgt_1_2".into(), json!("All Pitch"));
            p.insert("es_amt_1_2".into(), json!(if is_hat { 0.05 } else { 0.22 }));

            p.insert("x_smod".into(), json!("Soft"));
            p.insert("x_sdrv".into(), json!(0.24));
            p.insert("x_sdw".into(), json!(0.14));
            p.insert("x_rmod".into(), json!("Room"));
            p.insert("x_rdec".into(), json!(0.9));
            p.insert("x_rdw".into(), json!(0.12));
        }

        InstrumentRole::SoundFx => {
            p.insert("g_voc".into(), json!(8));
            p.insert("g_pmod".into(), json!("Poly"));
            p.insert("g_drft".into(), json!(true));

            let is_riser_or_wind = has_any(&text, &["riser", "wind", "exhaust", "vinyl", "downlifter"]);
            let is_fm_siren = has_any(&text, &["laser", "siren", "ping", "arc", "glitch", "warp"]);
            p.insert("o_mod_1".into(), json!(if is_riser_or_wind { "Noise" } else if is_fm_siren { "FM" } else { "Vowel" }));
            p.insert("o_wav_1".into(), json!(if is_fm_siren { "Metallic" } else { "Formant" }));
            p.insert("o_ca_1".into(), json!(0.50));
            p.insert("o_cb_1".into(), json!(0.42));
            p.insert("o_ucnt_1".into(), json!(3));
            p.insert("o_udet_1".into(), json!(0.30));
            p.insert("o_wid_1".into(), json!(0.78));
            p.insert("o_lvl_1".into(), json!(0.66));

            p.insert("o_mod_2".into(), json!("Classic"));
            p.insert("o_wav_2".into(), json!("Sub"));
            p.insert("o_oct_2".into(), json!(-1));
            p.insert("o_lvl_2".into(), json!(0.26));

            p.insert("f_mod_1".into(), json!("Perfecto"));
            p.insert("f_typ_1".into(), json!("LP2"));
            p.insert("f_cut_1".into(), json!(3800.0 + v * 2800.0));
            p.insert("f_res_1".into(), json!(0.28));

            p.insert("e_att_1".into(), json!(if text.contains("riser") { 0.14 } else { 0.005 }));
            p.insert("e_dec_1".into(), json!(0.55));
            p.insert("e_sus_1".into(), json!(0.65));
            p.insert("e_rel_1".into(), json!(0.35));

            p.insert("l_rat_1".into(), json!(1.4 + v * 3.0));
            p.insert("ls_tgt_1_1".into(), json!("Filter1 Cutoff"));
            p.insert("ls_amt_1_1".into(), json!(0.30));

            p.insert("x_pdw".into(), json!(0.20));
            p.insert("x_dsyn".into(), json!(true));
            p.insert("x_dpp".into(), json!(true));
            p.insert("x_dfb".into(), json!(0.42));
            p.insert("x_ddw".into(), json!(0.22));
            p.insert("x_rmod".into(), json!("Hall"));
            p.insert("x_rdec".into(), json!(3.2));
            p.insert("x_rdw".into(), json!(0.26));
        }

        InstrumentRole::Init => unreachable!(),
    }

    (out, role)
}
