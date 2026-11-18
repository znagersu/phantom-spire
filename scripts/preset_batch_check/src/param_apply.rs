use nih_plug::prelude::Enum;
use phantom_spire::params::{
    set_bool_param, set_enum_param, set_float_param, set_int_param, ArpMode, ArpVelMode,
    ChorusMode, FilterModel, FilterType, LfoWaveform, MicrotuningMode, ModSource, ModTarget,
    OscMode, OscWaveform, PhantomSpireParams, PolyMode, ReverbMode, ShaperMode, SyncDiv,
    UnisonMode,
};
use phantom_spire::presets::PresetData;
use phantom_spire::SharedSynthState;

fn val_f32(v: &serde_json::Value) -> Option<f32> {
    v.as_f64().filter(|x| x.is_finite()).map(|x| x as f32)
}

fn val_i32(v: &serde_json::Value) -> Option<i32> {
    v.as_i64()
        .or_else(|| v.as_f64().map(|x| x.round() as i64))
        .map(|x| x as i32)
}

fn val_bool(v: &serde_json::Value) -> Option<bool> {
    v.as_bool().or_else(|| v.as_i64().map(|n| n != 0))
}

fn enum_from_json<E: Enum + PartialEq + Copy + 'static>(v: &serde_json::Value) -> Option<E> {
    if let Some(s) = v.as_str() {
        let variants = E::variants();
        let ids = E::ids();
        (0..variants.len())
            .find(|&idx| {
                variants[idx].eq_ignore_ascii_case(s)
                    || ids
                        .and_then(|arr| arr.get(idx))
                        .map(|id| id.eq_ignore_ascii_case(s))
                        .unwrap_or(false)
            })
            .map(E::from_index)
    } else {
        v.as_i64().map(|idx| E::from_index(idx.max(0) as usize))
    }
}

fn set_mod_source_enum(param: &mut nih_plug::prelude::EnumParam<ModSource>, v: &serde_json::Value) {
    if let Some(m) = enum_from_json::<ModSource>(v) {
        set_enum_param(param, m);
    }
}

fn set_mod_target_enum(param: &mut nih_plug::prelude::EnumParam<ModTarget>, v: &serde_json::Value) {
    if let Some(m) = enum_from_json::<ModTarget>(v) {
        set_enum_param(param, m);
    }
}

fn parse_one_based_suffix(key: &str, prefix: &str, max_n: usize) -> Option<usize> {
    let rest = key.strip_prefix(prefix)?;
    let idx: usize = rest.parse().ok()?;
    if idx >= 1 && idx <= max_n {
        Some(idx - 1)
    } else {
        None
    }
}

fn parse_two_indices(key: &str, prefix: &str, max_a: usize, max_b: usize) -> Option<(usize, usize)> {
    let rest = key.strip_prefix(prefix)?;
    let (a_str, b_str) = rest.split_once('_')?;
    let a: usize = a_str.parse().ok()?;
    let b: usize = b_str.parse().ok()?;
    if a >= 1 && a <= max_a && b >= 1 && b <= max_b {
        Some((a - 1, b - 1))
    } else {
        None
    }
}

/// Apply every parameter and non-parameter state field from `PresetData` to a fresh
/// `PhantomSpireParams` + `SharedSynthState` so offline rendering matches live preset loading.
pub fn apply_preset_to_engine_state(
    data: &PresetData,
    shared_synth: &SharedSynthState,
) -> Result<PhantomSpireParams, String> {
    let mut params = PhantomSpireParams::default();

    // Sync non-parameter extra state (steppers, arp grid, mod matrix, microtuning)
    {
        let mut extra = params
            .extra_state
            .lock()
            .map_err(|_| String::from("extra_state mutex poisoned"))?;
        extra.preset_name = data.name.clone();
        extra.preset_category = data.category.clone();
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
        shared_synth.sync_from_extra(&extra);
    }

    for (key, val) in &data.params {
        // 1. Global params
        match key.as_str() {
            "g_voc" => if let Some(v) = val_i32(val) { set_int_param(&mut params.global.voices, v); continue; },
            "g_pmod" => if let Some(e) = enum_from_json::<PolyMode>(val) { set_enum_param(&mut params.global.poly_mode, e); continue; },
            "g_trns" => if let Some(v) = val_i32(val) { set_int_param(&mut params.global.transpose, v); continue; },
            "g_gld" => if let Some(v) = val_f32(val) { set_float_param(&mut params.global.glide, v); continue; },
            "g_glog" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.global.glide_log, v); continue; },
            "g_bup" => if let Some(v) = val_i32(val) { set_int_param(&mut params.global.bender_up, v); continue; },
            "g_bdn" => if let Some(v) = val_i32(val) { set_int_param(&mut params.global.bender_down, v); continue; },
            "g_drft" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.global.drift, v); continue; },
            "g_pw" => if let Some(v) = val_f32(val) { set_float_param(&mut params.global.pitch_wheel, v); continue; },
            "g_mw" => if let Some(v) = val_f32(val) { set_float_param(&mut params.global.mod_wheel, v); continue; },
            "g_utun" => if let Some(e) = enum_from_json::<MicrotuningMode>(val) { set_enum_param(&mut params.global.microtuning, e); continue; },
            "host_bpm_sync" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.global.host_bpm_sync, v); continue; },
            // Filter routing
            "fr_bal" => if let Some(v) = val_f32(val) { set_float_param(&mut params.filter_routing.filter_balance, v); continue; },
            "fr_par" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.filter_routing.parallel, v); continue; },
            "fr_lnk" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.filter_routing.link, v); continue; },
            // Arp params
            "a_on" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.arp.enabled, v); continue; },
            "a_mod" => if let Some(e) = enum_from_json::<ArpMode>(val) { set_enum_param(&mut params.arp.mode, e); continue; },
            "a_oct" => if let Some(v) = val_i32(val) { set_int_param(&mut params.arp.octaves, v); continue; },
            "a_rat" => if let Some(v) = val_f32(val) { set_float_param(&mut params.arp.rate, v); continue; },
            "a_syn" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.arp.sync, v); continue; },
            "a_sdiv" => if let Some(e) = enum_from_json::<SyncDiv>(val) { set_enum_param(&mut params.arp.sync_div, e); continue; },
            "a_gat" => if let Some(v) = val_f32(val) { set_float_param(&mut params.arp.gate, v); continue; },
            "a_swg" => if let Some(v) = val_f32(val) { set_float_param(&mut params.arp.swing, v); continue; },
            "a_lat" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.arp.latch, v); continue; },
            "a_vel" => if let Some(e) = enum_from_json::<ArpVelMode>(val) { set_enum_param(&mut params.arp.vel_mode, e); continue; },
            "a_cnt" => if let Some(v) = val_i32(val) { set_int_param(&mut params.arp.steps_count, v); continue; },
            "a_bpm" => if let Some(v) = val_f32(val) { set_float_param(&mut params.arp.internal_bpm, v); continue; },
            // Master params
            "k_xcmp" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.x_comp, v); continue; },
            "k_warm" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.master.warm, v); continue; },
            "k_soft" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.master.soft, v); continue; },
            "k_boos" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.master.boost, v); continue; },
            "k_eqon" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.master.eq_on, v); continue; },
            "k_elf" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_low_freq, v); continue; },
            "k_elq" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_low_q, v); continue; },
            "k_elg" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_low_gain, v); continue; },
            "k_emf" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_mid_freq, v); continue; },
            "k_emq" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_mid_q, v); continue; },
            "k_emg" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_mid_gain, v); continue; },
            "k_ehf" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_high_freq, v); continue; },
            "k_ehq" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_high_q, v); continue; },
            "k_ehg" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.eq_high_gain, v); continue; },
            "k_vel" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.velocity, v); continue; },
            "k_vol" => if let Some(v) = val_f32(val) { set_float_param(&mut params.master.volume, v); continue; },
            "k_lim" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.master.soft_limit, v); continue; },
            // FX Shaper
            "x_smut" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.shaper.mute, v); continue; },
            "x_smod" => if let Some(e) = enum_from_json::<ShaperMode>(val) { set_enum_param(&mut params.fx.shaper.mode, e); continue; },
            "x_sbnd" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.shaper.band, v); continue; },
            "x_shq" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.shaper.hq, v); continue; },
            "x_sdrv" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.shaper.drive, v); continue; },
            "x_sbit" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.shaper.bit, v); continue; },
            "x_ssrt" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.shaper.s_rate, v); continue; },
            "x_sloc" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.shaper.low_cut, v); continue; },
            "x_shic" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.shaper.hi_cut, v); continue; },
            "x_sdw" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.shaper.dry_wet, v); continue; },
            // FX Phaser
            "x_pmut" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.phaser.mute, v); continue; },
            "x_pstg" => if let Some(v) = val_i32(val) { set_int_param(&mut params.fx.phaser.stages, v); continue; },
            "x_prat" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.phaser.rate, v); continue; },
            "x_pfb" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.phaser.feedback, v); continue; },
            "x_pfrq" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.phaser.freq, v); continue; },
            "x_pspr" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.phaser.spread, v); continue; },
            "x_pdw" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.phaser.dry_wet, v); continue; },
            // FX Chorus
            "x_cmut" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.chorus.mute, v); continue; },
            "x_cmod" => if let Some(e) = enum_from_json::<ChorusMode>(val) { set_enum_param(&mut params.fx.chorus.mode, e); continue; },
            "x_crat" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.chorus.rate, v); continue; },
            "x_cdel" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.chorus.delay, v); continue; },
            "x_cdpt" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.chorus.depth, v); continue; },
            "x_cfb" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.chorus.feedback, v); continue; },
            "x_cloc" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.chorus.low_cut, v); continue; },
            "x_chic" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.chorus.hi_cut, v); continue; },
            "x_cdw" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.chorus.dry_wet, v); continue; },
            // FX Delay
            "x_dmut" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.delay.mute, v); continue; },
            "x_dsyn" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.delay.sync, v); continue; },
            "x_dtl" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.delay.time_l, v); continue; },
            "x_dtr" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.delay.time_r, v); continue; },
            "x_dsdl" => if let Some(e) = enum_from_json::<SyncDiv>(val) { set_enum_param(&mut params.fx.delay.sync_div_l, e); continue; },
            "x_dsdr" => if let Some(e) = enum_from_json::<SyncDiv>(val) { set_enum_param(&mut params.fx.delay.sync_div_r, e); continue; },
            "x_dfb" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.delay.feedback, v); continue; },
            "x_dpp" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.delay.ping_pong, v); continue; },
            "x_drat" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.delay.rate, v); continue; },
            "x_ddpt" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.delay.depth, v); continue; },
            "x_dloc" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.delay.low_cut, v); continue; },
            "x_dhic" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.delay.hi_cut, v); continue; },
            "x_ddw" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.delay.dry_wet, v); continue; },
            // FX Reverb
            "x_rmut" => if let Some(v) = val_bool(val) { set_bool_param(&mut params.fx.reverb.mute, v); continue; },
            "x_rmod" => if let Some(e) = enum_from_json::<ReverbMode>(val) { set_enum_param(&mut params.fx.reverb.mode, e); continue; },
            "x_rpre" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.reverb.predelay, v); continue; },
            "x_rdec" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.reverb.decay, v); continue; },
            "x_rdmp" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.reverb.damp, v); continue; },
            "x_rcol" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.reverb.color, v); continue; },
            "x_rwid" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.reverb.wide, v); continue; },
            "x_rdw" => if let Some(v) = val_f32(val) { set_float_param(&mut params.fx.reverb.dry_wet, v); continue; },
            _ => {}
        }

        // 2. Indexed array parameters
        if let Some(i) = parse_one_based_suffix(key, "g_mac_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.global.macros[i].value, v); }
            continue;
        }
        // Oscillators (1..4)
        if let Some(i) = parse_one_based_suffix(key, "o_mod_", 4) {
            if let Some(e) = enum_from_json::<OscMode>(val) { set_enum_param(&mut params.oscs[i].mode, e); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_wav_", 4) {
            if let Some(e) = enum_from_json::<OscWaveform>(val) { set_enum_param(&mut params.oscs[i].waveform, e); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_wtm_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].wt_mix, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_not_", 4) {
            if let Some(v) = val_i32(val) { set_int_param(&mut params.oscs[i].note, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_fin_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].fine, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_oct_", 4) {
            if let Some(v) = val_i32(val) { set_int_param(&mut params.oscs[i].octave, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_ca_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].ctrl_a, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_cb_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].ctrl_b, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_phs_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].phase, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_ucnt_", 4) {
            if let Some(v) = val_i32(val) { set_int_param(&mut params.oscs[i].unison_count, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_udet_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].unison_detune, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_udns_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].unison_density, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_umod_", 4) {
            if let Some(e) = enum_from_json::<UnisonMode>(val) { set_enum_param(&mut params.oscs[i].unison_mode, e); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_wid_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].wide, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_pan_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].pan, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_lvl_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].mix_level, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_fbal_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.oscs[i].filter_balance, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_ana_", 4) {
            if let Some(v) = val_bool(val) { set_bool_param(&mut params.oscs[i].ana, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_inv_", 4) {
            if let Some(v) = val_bool(val) { set_bool_param(&mut params.oscs[i].inv, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "o_key_", 4) {
            if let Some(v) = val_bool(val) { set_bool_param(&mut params.oscs[i].key, v); } continue;
        }

        // Filters (1..2)
        if let Some(i) = parse_one_based_suffix(key, "f_mod_", 2) {
            if let Some(e) = enum_from_json::<FilterModel>(val) { set_enum_param(&mut params.filters[i].model, e); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "f_typ_", 2) {
            if let Some(e) = enum_from_json::<FilterType>(val) { set_enum_param(&mut params.filters[i].filter_type, e); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "f_cut_", 2) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.filters[i].cutoff, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "f_res_", 2) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.filters[i].resonance, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "f_ktr_", 2) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.filters[i].keytrack, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "f_drv_", 2) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.filters[i].drive, v); } continue;
        }

        // Envelopes (1..4) and nested slots es_*_{slot}_{env}
        if let Some(i) = parse_one_based_suffix(key, "e_att_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[i].attack, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "e_dec_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[i].decay, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "e_sus_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[i].sustain, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "e_slt_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[i].slt, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "e_sll_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[i].sll, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "e_rel_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[i].release, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "e_vel_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[i].vel, v); } continue;
        }
        if let Some((slot, env)) = parse_two_indices(key, "es_tgt_", 2, 4) {
            set_mod_target_enum(&mut params.envs[env].mod_slots[slot].target, val);
            continue;
        }
        if let Some((slot, env)) = parse_two_indices(key, "es_amt_", 2, 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[env].mod_slots[slot].amt, v); }
            continue;
        }
        if let Some((slot, env)) = parse_two_indices(key, "es_vel_", 2, 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.envs[env].mod_slots[slot].vel, v); }
            continue;
        }

        // LFOs (1..4) and nested slots ls_*_{slot}_{lfo}
        if let Some(i) = parse_one_based_suffix(key, "l_rat_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.lfos[i].rate, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_syn_", 4) {
            if let Some(v) = val_bool(val) { set_bool_param(&mut params.lfos[i].sync, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_sdiv_", 4) {
            if let Some(e) = enum_from_json::<SyncDiv>(val) { set_enum_param(&mut params.lfos[i].sync_div, e); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_wav_", 4) {
            if let Some(e) = enum_from_json::<LfoWaveform>(val) { set_enum_param(&mut params.lfos[i].waveform, e); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_mrp_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.lfos[i].morph, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_sym_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.lfos[i].sym, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_phs_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.lfos[i].phase, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_fad_", 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.lfos[i].fade, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_uni_", 4) {
            if let Some(v) = val_bool(val) { set_bool_param(&mut params.lfos[i].unipolar, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "l_mon_", 4) {
            if let Some(v) = val_bool(val) { set_bool_param(&mut params.lfos[i].mono, v); } continue;
        }
        if let Some((slot, lfo)) = parse_two_indices(key, "ls_tgt_", 2, 4) {
            set_mod_target_enum(&mut params.lfos[lfo].mod_slots[slot].target, val);
            continue;
        }
        if let Some((slot, lfo)) = parse_two_indices(key, "ls_amt_", 2, 4) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.lfos[lfo].mod_slots[slot].amt, v); }
            continue;
        }

        // Steppers (1..2)
        if let Some(i) = parse_one_based_suffix(key, "s_rat_", 2) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.steppers[i].rate, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "s_syn_", 2) {
            if let Some(v) = val_bool(val) { set_bool_param(&mut params.steppers[i].sync, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "s_sdiv_", 2) {
            if let Some(e) = enum_from_json::<SyncDiv>(val) { set_enum_param(&mut params.steppers[i].sync_div, e); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "s_gat_", 2) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.steppers[i].gate, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "s_smt_", 2) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.steppers[i].smooth, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "s_cnt_", 2) {
            if let Some(v) = val_i32(val) { set_int_param(&mut params.steppers[i].steps_count, v); } continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "s_lop_", 2) {
            if let Some(v) = val_bool(val) { set_bool_param(&mut params.steppers[i].loop_mode, v); } continue;
        }

        // Mod matrix (1..15)
        if let Some(i) = parse_one_based_suffix(key, "m_src_", 15) {
            set_mod_source_enum(&mut params.mod_slots[i].source, val);
            continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "m_tgt_", 15) {
            set_mod_target_enum(&mut params.mod_slots[i].target, val);
            continue;
        }
        if let Some(i) = parse_one_based_suffix(key, "m_amt_", 15) {
            if let Some(v) = val_f32(val) { set_float_param(&mut params.mod_slots[i].amount, v); }
            continue;
        }

        return Err(format!("Unknown preset parameter key '{key}' in '{}'", data.name));
    }

    Ok(params)
}
