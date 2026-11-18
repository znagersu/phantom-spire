use phantom_spire::dsp::{SynthEngine, TransportInfo};
use phantom_spire::params::{
    set_bool_param, set_enum_param, set_float_param, set_int_param, ArpMode, FilterModel,
    FilterType, LfoWaveform, MicrotuningMode, ModSource, ModTarget, OscMode, OscWaveform,
    PhantomSpireParams, PolyMode, ShaperMode, UnisonMode,
};
use phantom_spire::presets::{PresetBank, PresetManager, SPIRE_CATEGORIES};
use phantom_spire::SharedSynthState;
use std::f64::consts::PI;

const SR: f32 = 48000.0;
const BASI_MAX_PEAK: f32 = 0.995;
const BASI_MAX_DC: f64 = 0.005;

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct RenderMetrics {
    nan_count: usize,
    inf_count: usize,
    subnormal_count: usize,
    peak: f32,
    dc_max_abs: f64,
    rms_l: f64,
    rms_r: f64,
    rms_mono: f64,
    stereo_diff_rms: f64,
    centroid_hz: f64,
}

fn analyze(left: &[f32], right: &[f32], sample_rate: f64) -> RenderMetrics {
    let n = left.len().min(right.len());
    let mut nan_count = 0usize;
    let mut inf_count = 0usize;
    let mut subnormal_count = 0usize;
    let mut peak = 0.0_f32;
    let mut sum_l = 0.0_f64;
    let mut sum_r = 0.0_f64;
    let mut sum_l2 = 0.0_f64;
    let mut sum_r2 = 0.0_f64;
    let mut sum_diff2 = 0.0_f64;

    for i in 0..n {
        let l = left[i];
        let r = right[i];
        for s in [l, r] {
            if s.is_nan() {
                nan_count += 1;
            } else if s.is_infinite() {
                inf_count += 1;
            } else if s != 0.0 && s.is_subnormal() {
                subnormal_count += 1;
            }
        }
        if l.is_finite() && r.is_finite() {
            peak = peak.max(l.abs()).max(r.abs());
            let ld = l as f64;
            let rd = r as f64;
            sum_l += ld;
            sum_r += rd;
            sum_l2 += ld * ld;
            sum_r2 += rd * rd;
            let d = ld - rd;
            sum_diff2 += d * d;
        }
    }

    let nf = n.max(1) as f64;
    let dc_max_abs = (sum_l / nf).abs().max((sum_r / nf).abs());
    let rms_l = (sum_l2 / nf).sqrt();
    let rms_r = (sum_r2 / nf).sqrt();
    let rms_mono = (0.5 * (rms_l * rms_l + rms_r * rms_r)).sqrt();
    let stereo_diff_rms = (sum_diff2 / nf).sqrt();
    let centroid_hz = estimate_centroid(left, right, sample_rate);

    RenderMetrics {
        nan_count,
        inf_count,
        subnormal_count,
        peak,
        dc_max_abs,
        rms_l,
        rms_r,
        rms_mono,
        stereo_diff_rms,
        centroid_hz,
    }
}

fn estimate_centroid(left: &[f32], right: &[f32], sr: f64) -> f64 {
    let n = left.len().min(right.len());
    if n < 512 {
        return 0.0;
    }
    let fft_len = 2048usize.min(n.next_power_of_two() / 2).max(256);
    let start = (n - fft_len) / 2;
    let mut re = vec![0.0_f64; fft_len];
    let mut im = vec![0.0_f64; fft_len];
    for i in 0..fft_len {
        let w = 0.5 * (1.0 - (2.0 * PI * i as f64 / (fft_len as f64 - 1.0)).cos());
        re[i] = 0.5 * (left[start + i] as f64 + right[start + i] as f64) * w;
    }
    // Radix-2 FFT
    let mut j = 0usize;
    for i in 1..fft_len {
        let mut bit = fft_len >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= fft_len {
        let ang = -2.0 * PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let half = len / 2;
        for i in (0..fft_len).step_by(len) {
            let (mut cr, mut ci) = (1.0_f64, 0.0_f64);
            for k in 0..half {
                let ur = re[i + k];
                let ui = im[i + k];
                let vr = re[i + k + half] * cr - im[i + k + half] * ci;
                let vi = re[i + k + half] * ci + im[i + k + half] * cr;
                re[i + k] = ur + vr;
                im[i + k] = ui + vi;
                re[i + k + half] = ur - vr;
                im[i + k + half] = ui - vi;
                let nr = cr * wr - ci * wi;
                let ni = cr * wi + ci * wr;
                cr = nr;
                ci = ni;
            }
        }
        len <<= 1;
    }
    let bin_hz = sr / fft_len as f64;
    let mut sum_m = 0.0_f64;
    let mut sum_fm = 0.0_f64;
    for k in 1..(fft_len / 2) {
        let m = (re[k] * re[k] + im[k] * im[k]).sqrt();
        sum_m += m;
        sum_fm += (k as f64 * bin_hz) * m;
    }
    if sum_m > 1e-12 {
        sum_fm / sum_m
    } else {
        0.0
    }
}

fn l2_diff(a: &(Vec<f32>, Vec<f32>), b: &(Vec<f32>, Vec<f32>)) -> f64 {
    let n = a.0.len().min(b.0.len()).min(a.1.len()).min(b.1.len());
    if n == 0 {
        return 0.0;
    }
    let mut acc = 0.0_f64;
    for i in 0..n {
        let dl = (a.0[i] - b.0[i]) as f64;
        let dr = (a.1[i] - b.1[i]) as f64;
        acc += 0.5 * (dl * dl + dr * dr);
    }
    (acc / n as f64).sqrt()
}

fn assert_basi(tag: &str, m: &RenderMetrics) {
    assert_eq!(m.nan_count, 0, "[{tag}] NaN samples detected: {}", m.nan_count);
    assert_eq!(m.inf_count, 0, "[{tag}] Inf samples detected: {}", m.inf_count);
    assert_eq!(
        m.subnormal_count, 0,
        "[{tag}] Subnormal samples detected: {}",
        m.subnormal_count
    );
    assert!(
        m.peak <= BASI_MAX_PEAK,
        "[{tag}] Peak {:.5} exceeds BASI limit {:.3}",
        m.peak,
        BASI_MAX_PEAK
    );
    assert!(
        m.dc_max_abs <= BASI_MAX_DC,
        "[{tag}] |DC| {:.5} exceeds BASI limit {:.3}",
        m.dc_max_abs,
        BASI_MAX_DC
    );
}

fn render_notes_with_sr(
    params: &PhantomSpireParams,
    shared: &SharedSynthState,
    notes: &[u8],
    hold_samples: usize,
    release_samples: usize,
    sr: f32,
) -> (Vec<f32>, Vec<f32>) {
    let mut engine = SynthEngine::new(sr);
    let transport = TransportInfo {
        tempo_bpm: 120.0,
        playing: true,
        sample_rate: sr,
        ppq_pos: Some(0.0),
        pos_samples: Some(0),
    };
    let vs = engine.make_voice_settings(params, shared);
    let vol = params.master.volume.value();

    for &n in notes {
        engine.note_on_with_params(n, 0.85, 0, params, &vs);
    }

    let total = hold_samples + release_samples;
    let mut out_l = Vec::with_capacity(total);
    let mut out_r = Vec::with_capacity(total);

    for i in 0..total {
        if i == hold_samples {
            for &n in notes {
                engine.note_off_with_params(n, 0, params, &vs);
            }
        }
        let (l, r) = engine.process_sample_full(params, shared, &transport, &vs, vol);
        out_l.push(l);
        out_r.push(r);
    }
    (out_l, out_r)
}

fn render_notes(
    params: &PhantomSpireParams,
    shared: &SharedSynthState,
    notes: &[u8],
    hold_samples: usize,
    release_samples: usize,
) -> (Vec<f32>, Vec<f32>) {
    render_notes_with_sr(params, shared, notes, hold_samples, release_samples, SR)
}

fn make_clean_init() -> (PhantomSpireParams, SharedSynthState) {
    let mut params = PhantomSpireParams::default();
    let shared = SharedSynthState::default();
    // Ensure clean deterministic starting point: OSC1 on (Saw), others off, filter open, FX muted
    set_float_param(&mut params.oscs[0].mix_level, 0.85);
    set_enum_param(&mut params.oscs[0].mode, OscMode::Classic);
    set_enum_param(&mut params.oscs[0].waveform, OscWaveform::Saw);
    set_bool_param(&mut params.oscs[0].ana, false);
    set_bool_param(&mut params.oscs[0].key, true);
    for i in 1..4 {
        set_float_param(&mut params.oscs[i].mix_level, 0.0);
        set_bool_param(&mut params.oscs[i].ana, false);
    }
    set_float_param(&mut params.filters[0].cutoff, 16000.0);
    set_float_param(&mut params.filters[0].resonance, 0.0);
    set_bool_param(&mut params.fx.shaper.mute, true);
    set_bool_param(&mut params.fx.phaser.mute, true);
    set_bool_param(&mut params.fx.chorus.mute, true);
    set_bool_param(&mut params.fx.delay.mute, true);
    set_bool_param(&mut params.fx.reverb.mute, true);
    set_bool_param(&mut params.master.eq_on, false);
    set_float_param(&mut params.master.x_comp, 0.0);
    set_bool_param(&mut params.master.warm, false);
    set_bool_param(&mut params.master.soft, false);
    set_bool_param(&mut params.master.boost, false);
    set_float_param(&mut params.master.volume, 0.8);
    if let Ok(extra) = params.extra_state.lock() {
        shared.sync_from_extra(&extra);
    }
    (params, shared)
}

#[test]
fn e2e_01_polyphonic_16_voices_and_multirate_basi_safety() {
    let (params, shared) = make_clean_init();
    let chord_16: [u8; 16] = [
        36, 40, 43, 48, 52, 55, 59, 60, 64, 67, 71, 72, 76, 79, 83, 84,
    ];
    for sr in [44100.0_f32, 48000.0, 96000.0] {
        let hold = (sr * 0.25) as usize;
        let rel = (sr * 0.15) as usize;
        let (l, r) = render_notes_with_sr(&params, &shared, &chord_16, hold, rel, sr);
        let m = analyze(&l, &r, sr as f64);
        assert_basi(&format!("16v@{sr}Hz"), &m);
        assert!(
            m.rms_mono > 0.02,
            "16-voice chord at {sr}Hz should be clearly audible, rms={}",
            m.rms_mono
        );
    }
}

#[test]
fn e2e_02_all_four_oscillators_and_all_seven_osc_modes() {
    // Verify each of OSC 1..4 produces audible BASI-safe audio independently
    for osc_idx in 0..4 {
        let (mut params, shared) = make_clean_init();
        for i in 0..4 {
            set_float_param(&mut params.oscs[i].mix_level, if i == osc_idx { 0.8 } else { 0.0 });
        }
        set_int_param(&mut params.oscs[osc_idx].octave, if osc_idx == 2 { 1 } else { 0 });
        let (l, r) = render_notes(&params, &shared, &[60], 8192, 2048);
        let m = analyze(&l, &r, SR as f64);
        assert_basi(&format!("OSC{}", osc_idx + 1), &m);
        assert!(
            m.rms_mono > 0.01,
            "OSC{} produced silent output: rms={}",
            osc_idx + 1,
            m.rms_mono
        );
    }

    // Verify all 7 OscModes produce BASI-safe audio and differ from Classic baseline
    let (base_p, shared) = make_clean_init();
    let base_buf = render_notes(&base_p, &shared, &[60], 8192, 1024);

    let modes = [
        OscMode::Classic,
        OscMode::Noise,
        OscMode::FM,
        OscMode::AMSync,
        OscMode::SawPWM,
        OscMode::HardFM,
        OscMode::Vowel,
    ];
    for mode in modes {
        let (mut p, s) = make_clean_init();
        set_enum_param(&mut p.oscs[0].mode, mode);
        set_float_param(&mut p.oscs[0].ctrl_a, 0.72);
        set_float_param(&mut p.oscs[0].ctrl_b, 0.58);
        set_float_param(&mut p.oscs[0].wt_mix, 0.6);
        set_enum_param(&mut p.oscs[0].waveform, OscWaveform::Formant);
        let buf = render_notes(&p, &s, &[60], 8192, 1024);
        let m = analyze(&buf.0, &buf.1, SR as f64);
        assert_basi(&format!("OscMode::{mode:?}"), &m);
        assert!(m.rms_mono > 0.01, "OscMode::{mode:?} was silent");
        if mode != OscMode::Classic {
            let diff = l2_diff(&base_buf, &buf);
            assert!(
                diff > 0.01,
                "OscMode::{mode:?} did not measurably differ from Classic (l2_diff={diff})"
            );
        }
    }
}

#[test]
fn e2e_03_unison_detune_stereo_width_and_osc_mix() {
    let (mut p_mono, shared) = make_clean_init();
    set_int_param(&mut p_mono.oscs[0].unison_count, 1);
    set_float_param(&mut p_mono.oscs[0].wide, 0.0);
    set_float_param(&mut p_mono.oscs[0].pan, 0.0);
    let buf_mono = render_notes(&p_mono, &shared, &[60], 10000, 1024);
    let m_mono = analyze(&buf_mono.0, &buf_mono.1, SR as f64);
    assert_basi("unison-1-mono", &m_mono);

    let (mut p_uni, _) = make_clean_init();
    set_int_param(&mut p_uni.oscs[0].unison_count, 7);
    set_float_param(&mut p_uni.oscs[0].unison_detune, 0.55);
    set_float_param(&mut p_uni.oscs[0].unison_density, 0.7);
    set_enum_param(&mut p_uni.oscs[0].unison_mode, UnisonMode::Fifth);
    set_float_param(&mut p_uni.oscs[0].wide, 0.9);
    let buf_uni = render_notes(&p_uni, &shared, &[60], 10000, 1024);
    let m_uni = analyze(&buf_uni.0, &buf_uni.1, SR as f64);
    assert_basi("unison-7-wide", &m_uni);
    assert!(
        m_uni.stereo_diff_rms > 0.01,
        "Unison wide=0.9 should generate real stereo spread (diff_rms={})",
        m_uni.stereo_diff_rms
    );
    assert!(
        l2_diff(&buf_mono, &buf_uni) > 0.02,
        "7-voice unison detune must audibly differ from 1-voice mono"
    );

    // Phase INV check
    let (mut p_inv, _) = make_clean_init();
    set_bool_param(&mut p_inv.oscs[0].inv, true);
    let buf_inv = render_notes(&p_inv, &shared, &[60], 10000, 1024);
    assert!(
        l2_diff(&buf_mono, &buf_inv) > 0.05,
        "OSC INV phase inversion must differ from non-inverted waveform"
    );
}

#[test]
fn e2e_04_dual_filters_all_models_par_serial_and_link() {
    let (base_p, shared) = make_clean_init();
    let open_buf = render_notes(&base_p, &shared, &[48], 10000, 1024);
    let open_m = analyze(&open_buf.0, &open_buf.1, SR as f64);
    assert_basi("filter-open", &open_m);

    // Test all 5 Spire filter models with low cutoff & resonance
    let models = [
        FilterModel::Perfecto,
        FilterModel::Infecto,
        FilterModel::Acido,
        FilterModel::Scorpio,
        FilterModel::Combo,
    ];
    let mut model_bufs = Vec::new();
    for model in models {
        let (mut p, _) = make_clean_init();
        set_enum_param(&mut p.filters[0].model, model);
        set_enum_param(&mut p.filters[0].filter_type, FilterType::LP4);
        set_float_param(&mut p.filters[0].cutoff, 600.0);
        set_float_param(&mut p.filters[0].resonance, 0.55);
        set_float_param(&mut p.filter_routing.filter_balance, -1.0);
        let buf = render_notes(&p, &shared, &[48], 10000, 1024);
        let m = analyze(&buf.0, &buf.1, SR as f64);
        assert_basi(&format!("FilterModel::{model:?}"), &m);
        assert!(m.rms_mono > 0.005, "FilterModel::{model:?} went silent");
        assert!(
            l2_diff(&open_buf, &buf) > 0.02,
            "FilterModel::{model:?} at 600Hz must audibly filter signal"
        );
        model_bufs.push((model, buf));
    }

    // Verify models are distinct from one another (at least Perfecto vs Acido & Combo)
    let diff_perf_acido = l2_diff(&model_bufs[0].1, &model_bufs[2].1);
    assert!(
        diff_perf_acido > 0.005,
        "Perfecto and Acido should have distinct filter character (diff={diff_perf_acido})"
    );

    // PAR vs Serial routing difference
    let (mut p_ser, _) = make_clean_init();
    set_float_param(&mut p_ser.filters[0].cutoff, 1400.0);
    set_float_param(&mut p_ser.filters[1].cutoff, 500.0);
    set_enum_param(&mut p_ser.filters[1].filter_type, FilterType::HP2);
    set_bool_param(&mut p_ser.filter_routing.parallel, false);
    set_float_param(&mut p_ser.filter_routing.filter_balance, 0.0);
    let buf_ser = render_notes(&p_ser, &shared, &[48], 10000, 1024);
    assert_basi("filter-serial", &analyze(&buf_ser.0, &buf_ser.1, SR as f64));

    let (mut p_par, _) = make_clean_init();
    set_float_param(&mut p_par.filters[0].cutoff, 1400.0);
    set_float_param(&mut p_par.filters[1].cutoff, 500.0);
    set_enum_param(&mut p_par.filters[1].filter_type, FilterType::HP2);
    set_bool_param(&mut p_par.filter_routing.parallel, true);
    set_float_param(&mut p_par.filter_routing.filter_balance, 0.0);
    let buf_par = render_notes(&p_par, &shared, &[48], 10000, 1024);
    assert_basi("filter-parallel", &analyze(&buf_par.0, &buf_par.1, SR as f64));
    assert!(
        l2_diff(&buf_ser, &buf_par) > 0.01,
        "Parallel vs Serial dual-filter routing must produce measurable audio difference"
    );

    // LINK mode: Filter 2 follows Filter 1 cutoff (route OSC1 -> Filter 2 via oscs[0].filter_balance = 1.0)
    let (mut p_link_off, _) = make_clean_init();
    set_float_param(&mut p_link_off.oscs[0].filter_balance, 1.0);
    set_float_param(&mut p_link_off.filters[0].cutoff, 400.0);
    set_float_param(&mut p_link_off.filters[1].cutoff, 15000.0);
    set_float_param(&mut p_link_off.filter_routing.filter_balance, 1.0); // output F2
    set_bool_param(&mut p_link_off.filter_routing.parallel, true);
    set_bool_param(&mut p_link_off.filter_routing.link, false);
    let buf_link_off = render_notes(&p_link_off, &shared, &[48], 10000, 1024);

    let (mut p_link_on, _) = make_clean_init();
    set_float_param(&mut p_link_on.oscs[0].filter_balance, 1.0);
    set_float_param(&mut p_link_on.filters[0].cutoff, 400.0);
    set_float_param(&mut p_link_on.filters[1].cutoff, 15000.0);
    set_float_param(&mut p_link_on.filter_routing.filter_balance, 1.0);
    set_bool_param(&mut p_link_on.filter_routing.parallel, true);
    set_bool_param(&mut p_link_on.filter_routing.link, true);
    let buf_link_on = render_notes(&p_link_on, &shared, &[48], 10000, 1024);
    assert_basi("filter-link", &analyze(&buf_link_on.0, &buf_link_on.1, SR as f64));
    assert!(
        l2_diff(&buf_link_off, &buf_link_on) > 0.01,
        "Filter LINK must measurably couple Filter 2 cutoff to Filter 1"
    );
}

#[test]
fn e2e_05_voice_allocation_glide_microtuning_and_drift() {
    let (base_p, shared) = make_clean_init();
    let buf_eq = render_notes(&base_p, &shared, &[61, 65], 10000, 1024);

    // Microtuning difference (Equal vs QuarterTone)
    let (mut p_micro, _) = make_clean_init();
    set_enum_param(&mut p_micro.global.microtuning, MicrotuningMode::QuarterTone);
    let buf_micro = render_notes(&p_micro, &shared, &[61, 65], 10000, 1024);
    assert_basi("microtuning", &analyze(&buf_micro.0, &buf_micro.1, SR as f64));
    assert!(
        l2_diff(&buf_eq, &buf_micro) > 0.01,
        "Microtuning QuarterTone must measurably shift pitch vs Equal"
    );

    // Mono + Glide test (play C3 then C5 overlapping)
    let (mut p_glide, _) = make_clean_init();
    set_enum_param(&mut p_glide.global.poly_mode, PolyMode::Legato);
    set_float_param(&mut p_glide.global.glide, 250.0);
    set_bool_param(&mut p_glide.global.glide_log, true);
    set_bool_param(&mut p_glide.global.drift, true);
    let mut eng = SynthEngine::new(SR);
    let tr = TransportInfo {
        tempo_bpm: 120.0,
        playing: true,
        sample_rate: SR,
        ppq_pos: Some(0.0),
        pos_samples: Some(0),
    };
    let vs = eng.make_voice_settings(&p_glide, &shared);
    let vol = p_glide.master.volume.value();
    eng.note_on_with_params(48, 0.9, 0, &p_glide, &vs);
    let mut l = Vec::new();
    let mut r = Vec::new();
    for i in 0..14000 {
        if i == 4000 {
            eng.note_on_with_params(72, 0.9, 0, &p_glide, &vs);
        }
        let (sl, sr_s) = eng.process_sample_full(&p_glide, &shared, &tr, &vs, vol);
        l.push(sl);
        r.push(sr_s);
    }
    let m = analyze(&l, &r, SR as f64);
    assert_basi("mono-glide-drift", &m);
    assert!(m.rms_mono > 0.02);
}

#[test]
fn e2e_06_envelopes_lfos_steppers_arp_and_mod_matrix_macros() {
    let (base_p, shared) = make_clean_init();
    let base_buf = render_notes(&base_p, &shared, &[60], 16000, 2000);

    // 1. ENV2 -> Filter1Cutoff direct mod slot
    let (mut p_env, _) = make_clean_init();
    set_float_param(&mut p_env.filters[0].cutoff, 400.0);
    let env_dry = render_notes(&p_env, &shared, &[60], 16000, 2000);
    set_enum_param(&mut p_env.envs[1].mod_slots[0].target, ModTarget::Filter1Cutoff);
    set_float_param(&mut p_env.envs[1].mod_slots[0].amt, 0.9);
    set_float_param(&mut p_env.envs[1].attack, 0.01);
    set_float_param(&mut p_env.envs[1].decay, 0.15);
    let env_wet = render_notes(&p_env, &shared, &[60], 16000, 2000);
    assert_basi("env2-mod", &analyze(&env_wet.0, &env_wet.1, SR as f64));
    assert!(
        l2_diff(&env_dry, &env_wet) > 0.01,
        "ENV2 direct mod slot targeting Filter1Cutoff must produce measurable diff"
    );

    // 2. LFO1 -> AllOscPitch vibrato
    let (mut p_lfo, _) = make_clean_init();
    set_float_param(&mut p_lfo.lfos[0].rate, 8.0);
    set_bool_param(&mut p_lfo.lfos[0].sync, false);
    set_enum_param(&mut p_lfo.lfos[0].waveform, LfoWaveform::Sin);
    set_enum_param(&mut p_lfo.lfos[0].mod_slots[0].target, ModTarget::AllOscPitch);
    set_float_param(&mut p_lfo.lfos[0].mod_slots[0].amt, 0.6);
    let lfo_buf = render_notes(&p_lfo, &shared, &[60], 16000, 2000);
    assert_basi("lfo1-vibrato", &analyze(&lfo_buf.0, &lfo_buf.1, SR as f64));
    assert!(
        l2_diff(&base_buf, &lfo_buf) > 0.02,
        "LFO1 pitch modulation must produce measurable diff"
    );

    // 3. STP1 -> Filter1Cutoff via Mod Matrix
    let (mut p_stp, s_stp) = make_clean_init();
    set_float_param(&mut p_stp.filters[0].cutoff, 600.0);
    let stp_dry = render_notes(&p_stp, &s_stp, &[60], 20000, 1000);
    {
        let mut ex = p_stp.extra_state.lock().unwrap();
        ex.stepper_steps[0] = [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        s_stp.sync_from_extra(&ex);
    }
    set_bool_param(&mut p_stp.steppers[0].sync, false);
    set_float_param(&mut p_stp.steppers[0].rate, 12.0);
    set_enum_param(&mut p_stp.mod_slots[0].source, ModSource::Stp1);
    set_enum_param(&mut p_stp.mod_slots[0].target, ModTarget::Filter1Cutoff);
    set_float_param(&mut p_stp.mod_slots[0].amount, 0.85);
    let stp_wet = render_notes(&p_stp, &s_stp, &[60], 20000, 1000);
    assert_basi("stp1-mod-matrix", &analyze(&stp_wet.0, &stp_wet.1, SR as f64));
    assert!(
        l2_diff(&stp_dry, &stp_wet) > 0.01,
        "STP1 via Mod Matrix must produce measurable audio diff"
    );

    // 4. Macro 1 -> Filter1Cutoff via Mod Matrix
    let (mut p_mac, _) = make_clean_init();
    set_float_param(&mut p_mac.filters[0].cutoff, 500.0);
    set_enum_param(&mut p_mac.mod_slots[1].source, ModSource::Macro1);
    set_enum_param(&mut p_mac.mod_slots[1].target, ModTarget::Filter1Cutoff);
    set_float_param(&mut p_mac.mod_slots[1].amount, 0.9);
    set_float_param(&mut p_mac.global.macros[0].value, 0.0);
    let mac_lo = render_notes(&p_mac, &shared, &[60], 10000, 1000);
    set_float_param(&mut p_mac.global.macros[0].value, 1.0);
    let mac_hi = render_notes(&p_mac, &shared, &[60], 10000, 1000);
    assert_basi("macro1-hi", &analyze(&mac_hi.0, &mac_hi.1, SR as f64));
    assert!(
        l2_diff(&mac_lo, &mac_hi) > 0.02,
        "Macro 1 modulating Filter1Cutoff must produce measurable diff"
    );

    // 5. Pre-voice Arpeggiator
    let (mut p_arp, _) = make_clean_init();
    let arp_off = render_notes(&p_arp, &shared, &[60, 64, 67], 24000, 2000);
    set_bool_param(&mut p_arp.arp.enabled, true);
    set_enum_param(&mut p_arp.arp.mode, ArpMode::Up);
    set_int_param(&mut p_arp.arp.octaves, 2);
    set_bool_param(&mut p_arp.arp.sync, false);
    set_float_param(&mut p_arp.arp.rate, 10.0);
    set_float_param(&mut p_arp.arp.gate, 0.5);
    let arp_on = render_notes(&p_arp, &shared, &[60, 64, 67], 24000, 2000);
    let m_arp = analyze(&arp_on.0, &arp_on.1, SR as f64);
    assert_basi("arp-enabled", &m_arp);
    assert!(m_arp.rms_mono > 0.01, "Arpeggiator produced silent output");
    assert!(
        l2_diff(&arp_off, &arp_on) > 0.02,
        "Arpeggiator ON must measurably differ from static held chord"
    );
}

#[test]
fn e2e_07_all_five_fx_rack_modules_and_mute_switches() {
    let (dry_p, shared) = make_clean_init();
    let dry_buf = render_notes(&dry_p, &shared, &[60, 67], 16000, 8000);
    assert_basi("fx-dry", &analyze(&dry_buf.0, &dry_buf.1, SR as f64));

    // 1. SHAPER (SHP)
    let (mut p_shp, _) = make_clean_init();
    set_bool_param(&mut p_shp.fx.shaper.mute, false);
    set_enum_param(&mut p_shp.fx.shaper.mode, ShaperMode::Clip);
    set_float_param(&mut p_shp.fx.shaper.drive, 0.85);
    set_float_param(&mut p_shp.fx.shaper.dry_wet, 0.85);
    let buf_shp = render_notes(&p_shp, &shared, &[60, 67], 16000, 8000);
    assert_basi("fx-shaper", &analyze(&buf_shp.0, &buf_shp.1, SR as f64));
    assert!(l2_diff(&dry_buf, &buf_shp) > 0.01, "SHP must audibly shape audio");
    set_bool_param(&mut p_shp.fx.shaper.mute, true);
    let buf_shp_muted = render_notes(&p_shp, &shared, &[60, 67], 16000, 8000);
    assert!(l2_diff(&dry_buf, &buf_shp_muted) < 1e-4, "SHP MUTE must bypass cleanly");

    // 2. PHASER (PHS)
    let (mut p_phs, _) = make_clean_init();
    set_bool_param(&mut p_phs.fx.phaser.mute, false);
    set_int_param(&mut p_phs.fx.phaser.stages, 6);
    set_float_param(&mut p_phs.fx.phaser.feedback, 0.75);
    set_float_param(&mut p_phs.fx.phaser.rate, 3.0);
    set_float_param(&mut p_phs.fx.phaser.dry_wet, 0.8);
    let buf_phs = render_notes(&p_phs, &shared, &[60, 67], 16000, 8000);
    assert_basi("fx-phaser", &analyze(&buf_phs.0, &buf_phs.1, SR as f64));
    assert!(l2_diff(&dry_buf, &buf_phs) > 0.01, "PHS must audibly phase audio");

    // 3. CHORUS (CHR)
    let (mut p_chr, _) = make_clean_init();
    set_bool_param(&mut p_chr.fx.chorus.mute, false);
    set_float_param(&mut p_chr.fx.chorus.depth, 0.8);
    set_float_param(&mut p_chr.fx.chorus.rate, 2.5);
    set_float_param(&mut p_chr.fx.chorus.dry_wet, 0.8);
    let buf_chr = render_notes(&p_chr, &shared, &[60, 67], 16000, 8000);
    assert_basi("fx-chorus", &analyze(&buf_chr.0, &buf_chr.1, SR as f64));
    assert!(l2_diff(&dry_buf, &buf_chr) > 0.01, "CHR must audibly modulate audio");

    // 4. DELAY (DEL)
    let (mut p_del, _) = make_clean_init();
    set_bool_param(&mut p_del.fx.delay.mute, false);
    set_bool_param(&mut p_del.fx.delay.sync, false);
    set_float_param(&mut p_del.fx.delay.time_l, 0.08);
    set_float_param(&mut p_del.fx.delay.time_r, 0.12);
    set_bool_param(&mut p_del.fx.delay.ping_pong, true);
    set_float_param(&mut p_del.fx.delay.feedback, 0.6);
    set_float_param(&mut p_del.fx.delay.dry_wet, 0.7);
    let buf_del = render_notes(&p_del, &shared, &[60, 67], 16000, 8000);
    assert_basi("fx-delay", &analyze(&buf_del.0, &buf_del.1, SR as f64));
    assert!(l2_diff(&dry_buf, &buf_del) > 0.01, "DEL must audibly add echoes");

    // 5. REVERB (REV)
    let (mut p_rev, _) = make_clean_init();
    set_bool_param(&mut p_rev.fx.reverb.mute, false);
    set_float_param(&mut p_rev.fx.reverb.decay, 0.8);
    set_float_param(&mut p_rev.fx.reverb.dry_wet, 0.75);
    let buf_rev = render_notes(&p_rev, &shared, &[60, 67], 16000, 8000);
    assert_basi("fx-reverb", &analyze(&buf_rev.0, &buf_rev.1, SR as f64));
    assert!(l2_diff(&dry_buf, &buf_rev) > 0.01, "REV must audibly add reverb tail");
}

#[test]
fn e2e_08_master_eq_xcomp_and_warm_soft_boost() {
    let (dry_p, shared) = make_clean_init();
    let dry_buf = render_notes(&dry_p, &shared, &[52, 59], 12000, 2000);

    // X-Comp
    let (mut p_xc, _) = make_clean_init();
    set_float_param(&mut p_xc.master.x_comp, 0.85);
    let buf_xc = render_notes(&p_xc, &shared, &[52, 59], 12000, 2000);
    assert_basi("master-xcomp", &analyze(&buf_xc.0, &buf_xc.1, SR as f64));
    assert!(l2_diff(&dry_buf, &buf_xc) > 0.01, "Master X-Comp must measurably process audio");

    // Warm + Soft + Boost
    let (mut p_char, _) = make_clean_init();
    set_bool_param(&mut p_char.master.warm, true);
    set_bool_param(&mut p_char.master.soft, true);
    set_bool_param(&mut p_char.master.boost, true);
    let buf_char = render_notes(&p_char, &shared, &[52, 59], 12000, 2000);
    assert_basi("master-warm-soft-boost", &analyze(&buf_char.0, &buf_char.1, SR as f64));
    assert!(
        l2_diff(&dry_buf, &buf_char) > 0.01,
        "Master Warm/Soft/Boost must measurably color audio"
    );

    // 3-band EQ
    let (mut p_eq, _) = make_clean_init();
    set_bool_param(&mut p_eq.master.eq_on, true);
    set_float_param(&mut p_eq.master.eq_low_gain, 9.0);
    set_float_param(&mut p_eq.master.eq_high_gain, -9.0);
    let buf_eq = render_notes(&p_eq, &shared, &[52, 59], 12000, 2000);
    let m_eq = analyze(&buf_eq.0, &buf_eq.1, SR as f64);
    assert_basi("master-eq", &m_eq);
    assert!(l2_diff(&dry_buf, &buf_eq) > 0.01, "Master 3-band EQ must measurably shape spectrum");
}

#[test]
fn e2e_09_all_factory_presets_render_safely_and_user_preset_undo_redo() {
    use nih_plug::prelude::Params;
    use phantom_spire::params::ExtraSynthState;
    use std::collections::HashSet;

    let tmp_dir = std::env::temp_dir().join(format!(
        "phantom_spire_preset_e2e_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let mut pm = PresetManager::with_user_dir(tmp_dir.clone());
    assert!(
        pm.entries().len() >= 280,
        "Expected >=280 embedded Spire factory presets, got {}",
        pm.entries().len()
    );

    // Verify all 10 canonical Spire folder categories are represented
    let counts = pm.category_counts(PresetBank::Factory, "");
    for cat in SPIRE_CATEGORIES {
        let count = counts.iter().find(|(c, _)| c == cat).map(|(_, n)| *n).unwrap_or(0);
        assert!(
            count >= 1,
            "Canonical Spire category '{cat}' has 0 factory presets"
        );
    }

    // Verify author and tag metadata queries return non-empty results
    assert!(!pm.authors(PresetBank::Factory).is_empty(), "Expected non-empty preset authors");
    assert!(!pm.tags(PresetBank::Factory).is_empty(), "Expected non-empty preset tags");

    let params = PhantomSpireParams::default();
    let valid_ids: HashSet<String> = params.param_map().into_iter().map(|(id, _, _)| id).collect();
    let shared = SharedSynthState::default();
    let mut param_map = PresetManager::capture_param_values(&params);
    let mut extra = ExtraSynthState::default();

    for idx in 0..pm.entries().len() {
        let name = pm.entries()[idx].name.clone();
        let preset_keys: Vec<String> = pm.entries()[idx].data.params.keys().cloned().collect();
        for key in &preset_keys {
            assert!(
                valid_ids.contains(key),
                "Factory preset '{name}' references unknown param ID '{key}'"
            );
        }
        pm.apply_to_headless(idx, &mut param_map, &mut extra, &shared)
            .unwrap_or_else(|e| panic!("Failed to apply factory preset '{name}': {e}"));
        let (l, r) = render_notes(&params, &shared, &[55, 62, 67], 2048, 512);
        let m = analyze(&l, &r, SR as f64);
        assert_basi(&format!("Preset[{idx}:{name}]"), &m);
        assert!(
            m.rms_mono > 0.001,
            "Factory preset '{name}' produced near-silent audio (rms={})",
            m.rms_mono
        );
    }

    // Verify category/search/favorite filtering and backwards-compatible category normalization ("Lead" -> "Leads")
    pm.set_active_bank(PresetBank::Factory);
    pm.set_active_category(Some("Basses"));
    let bass_indices = pm.filtered_indices();
    assert!(bass_indices.len() >= 40, "Expected >=40 Basses presets, got {}", bass_indices.len());
    let first_bass = bass_indices[0];
    let fav_state = pm.toggle_favorite(first_bass);
    assert!(fav_state, "toggle_favorite should set true");
    pm.set_active_bank(PresetBank::Favorites);
    pm.set_active_category(None);
    assert!(pm.filtered_indices().contains(&first_bass));
    pm.set_active_bank(PresetBank::All);

    let captured = PresetManager::capture_from_params(&params);
    pm.save_user_preset_with_category("E2E_Custom_Test", "Lead", captured)
        .expect("save_user_preset_with_category should succeed");
    let pm_reloaded = PresetManager::with_user_dir(tmp_dir.clone());
    assert!(
        pm_reloaded
            .entries()
            .iter()
            .any(|e| e.name == "E2E_Custom_Test" && !e.is_factory && e.category == "Leads"),
        "Saved user preset was not reloaded from disk with normalized 'Leads' category"
    );

    assert!(pm.can_undo(), "Undo stack should be populated after preset switches");
    assert!(
        pm.undo_headless(&mut param_map, &mut extra, &shared),
        "undo_headless should succeed"
    );
    assert!(pm.can_redo(), "Redo stack should be populated after undo");
    assert!(
        pm.redo_headless(&mut param_map, &mut extra, &shared),
        "redo_headless should succeed"
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn e2e_10_project_bpm_sync_scales_arp_lfo_stepper_and_delay() {
    let (mut params, shared) = make_clean_init();
    assert!(params.global.host_bpm_sync.value(), "host_bpm_sync should default to true");

    set_bool_param(&mut params.arp.enabled, true);
    set_bool_param(&mut params.arp.sync, true);
    set_bool_param(&mut params.lfos[0].sync, true);
    set_enum_param(&mut params.lfos[0].mod_slots[0].target, ModTarget::Filter1Cutoff);
    set_float_param(&mut params.lfos[0].mod_slots[0].amt, 0.7);
    set_bool_param(&mut params.fx.delay.mute, false);
    set_bool_param(&mut params.fx.delay.sync, true);
    set_float_param(&mut params.fx.delay.dry_wet, 0.5);

    let render_at_bpm = |bpm: f32| -> (Vec<f32>, Vec<f32>) {
        let mut eng = SynthEngine::new(SR);
        let tr = TransportInfo {
            tempo_bpm: bpm,
            playing: true,
            sample_rate: SR,
            ppq_pos: Some(0.0),
            pos_samples: Some(0),
        };
        let vs = eng.make_voice_settings(&params, &shared);
        let vol = params.master.volume.value();
        eng.note_on_with_params(60, 0.85, 0, &params, &vs);
        eng.note_on_with_params(64, 0.85, 0, &params, &vs);
        let n = (SR * 0.6) as usize;
        let mut l = Vec::with_capacity(n);
        let mut r = Vec::with_capacity(n);
        for _ in 0..n {
            let (sl, sr_s) = eng.process_sample_full(&params, &shared, &tr, &vs, vol);
            l.push(sl);
            r.push(sr_s);
        }
        (l, r)
    };

    let buf_90 = render_at_bpm(90.0);
    let buf_160 = render_at_bpm(160.0);
    let m90 = analyze(&buf_90.0, &buf_90.1, SR as f64);
    let m160 = analyze(&buf_160.0, &buf_160.1, SR as f64);
    assert_basi("bpm-90", &m90);
    assert_basi("bpm-160", &m160);
    assert!(
        l2_diff(&buf_90, &buf_160) > 0.02,
        "Synced Arp+LFO+Delay must measurably scale between 90 BPM and 160 BPM"
    );
}
