use serde::Serialize;
use std::f64::consts::PI;

#[derive(Debug, Clone, Serialize)]
pub struct AudioMetrics {
    pub num_samples: usize,
    pub sample_rate: f64,
    pub nan_count: usize,
    pub inf_count: usize,
    pub subnormal_count: usize,
    pub peak_l: f32,
    pub peak_r: f32,
    pub peak_max: f32,
    pub dc_l: f64,
    pub dc_r: f64,
    pub dc_max_abs: f64,
    pub rms_l: f64,
    pub rms_r: f64,
    pub rms_dbfs: f64,
    pub stereo_diff_rms: f64,
    pub lr_correlation: f64,
    pub dominant_freq_hz: f64,
    pub spectral_centroid_hz: f64,
    pub basi_pass: bool,
    pub basi_errors: Vec<String>,
}

pub const BASI_MAX_PEAK: f32 = 0.995;
pub const BASI_MAX_DC: f64 = 0.005;

pub fn analyze_stereo(left: &[f32], right: &[f32], sample_rate: f64) -> AudioMetrics {
    let n = left.len().min(right.len());
    let mut nan_count = 0usize;
    let mut inf_count = 0usize;
    let mut subnormal_count = 0usize;
    let mut peak_l = 0.0_f32;
    let mut peak_r = 0.0_f32;
    let mut sum_l = 0.0_f64;
    let mut sum_r = 0.0_f64;
    let mut sum_l2 = 0.0_f64;
    let mut sum_r2 = 0.0_f64;
    let mut sum_lr = 0.0_f64;
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
        if l.is_finite() {
            peak_l = peak_l.max(l.abs());
            let ld = l as f64;
            sum_l += ld;
            sum_l2 += ld * ld;
        }
        if r.is_finite() {
            peak_r = peak_r.max(r.abs());
            let rd = r as f64;
            sum_r += rd;
            sum_r2 += rd * rd;
        }
        if l.is_finite() && r.is_finite() {
            let ld = l as f64;
            let rd = r as f64;
            sum_lr += ld * rd;
            let d = ld - rd;
            sum_diff2 += d * d;
        }
    }

    let nf = (n.max(1)) as f64;
    let dc_l = sum_l / nf;
    let dc_r = sum_r / nf;
    let dc_max_abs = dc_l.abs().max(dc_r.abs());
    let rms_l = (sum_l2 / nf).sqrt();
    let rms_r = (sum_r2 / nf).sqrt();
    let rms_mono = (0.5 * (rms_l * rms_l + rms_r * rms_r)).sqrt();
    let rms_dbfs = if rms_mono > 1e-12 {
        20.0 * rms_mono.log10()
    } else {
        -120.0
    };
    let stereo_diff_rms = (sum_diff2 / nf).sqrt();
    let denom = (sum_l2 * sum_r2).sqrt();
    let lr_correlation = if denom > 1e-12 {
        (sum_lr / denom).clamp(-1.0, 1.0)
    } else {
        1.0
    };

    let (dominant_freq_hz, spectral_centroid_hz) = estimate_spectrum(left, right, sample_rate);
    let peak_max = peak_l.max(peak_r);

    let mut basi_errors = Vec::new();
    if nan_count > 0 {
        basi_errors.push(format!("NaN samples detected: {}", nan_count));
    }
    if inf_count > 0 {
        basi_errors.push(format!("Inf samples detected: {}", inf_count));
    }
    if subnormal_count > 0 {
        basi_errors.push(format!("Subnormal samples detected: {}", subnormal_count));
    }
    if peak_max > BASI_MAX_PEAK {
        basi_errors.push(format!("Peak {:.5} exceeds BASI limit {:.3}", peak_max, BASI_MAX_PEAK));
    }
    if dc_max_abs > BASI_MAX_DC {
        basi_errors.push(format!("|DC| {:.5} exceeds BASI limit {:.3}", dc_max_abs, BASI_MAX_DC));
    }

    AudioMetrics {
        num_samples: n,
        sample_rate,
        nan_count,
        inf_count,
        subnormal_count,
        peak_l,
        peak_r,
        peak_max,
        dc_l,
        dc_r,
        dc_max_abs,
        rms_l,
        rms_r,
        rms_dbfs,
        stereo_diff_rms,
        lr_correlation,
        dominant_freq_hz,
        spectral_centroid_hz,
        basi_pass: basi_errors.is_empty(),
        basi_errors,
    }
}

/// Fast radix-2 FFT on a Hann-windowed mono slice (4096 samples) to estimate dominant frequency & centroid.
fn estimate_spectrum(left: &[f32], right: &[f32], sample_rate: f64) -> (f64, f64) {
    let n_avail = left.len().min(right.len());
    if n_avail < 256 || sample_rate <= 0.0 {
        return (0.0, 0.0);
    }
    let fft_len = 4096usize.min(n_avail.next_power_of_two() / 2).max(256);
    let start = if n_avail > fft_len { (n_avail - fft_len) / 2 } else { 0 };
    let mut re = vec![0.0_f64; fft_len];
    let mut im = vec![0.0_f64; fft_len];

    for i in 0..fft_len {
        let idx = start + i;
        let l = if idx < left.len() && left[idx].is_finite() { left[idx] as f64 } else { 0.0 };
        let r = if idx < right.len() && right[idx].is_finite() { right[idx] as f64 } else { 0.0 };
        let w = 0.5 * (1.0 - (2.0 * PI * i as f64 / (fft_len as f64 - 1.0)).cos());
        re[i] = 0.5 * (l + r) * w;
    }

    fft_inplace(&mut re, &mut im);

    let half = fft_len / 2;
    let bin_hz = sample_rate / fft_len as f64;
    let mut max_mag = 0.0_f64;
    let mut max_bin = 0usize;
    let mut sum_mag = 0.0_f64;
    let mut sum_freq_mag = 0.0_f64;

    for k in 1..half {
        let mag = (re[k] * re[k] + im[k] * im[k]).sqrt();
        let f = k as f64 * bin_hz;
        sum_mag += mag;
        sum_freq_mag += f * mag;
        if mag > max_mag {
            max_mag = mag;
            max_bin = k;
        }
    }

    let dom_hz = if max_bin > 0 && max_bin + 1 < half && max_mag > 1e-9 {
        let m_prev = (re[max_bin - 1].powi(2) + im[max_bin - 1].powi(2)).sqrt();
        let m_curr = max_mag;
        let m_next = (re[max_bin + 1].powi(2) + im[max_bin + 1].powi(2)).sqrt();
        let denom = m_prev - 2.0 * m_curr + m_next;
        let delta = if denom.abs() > 1e-12 {
            (0.5 * (m_prev - m_next) / denom).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        (max_bin as f64 + delta) * bin_hz
    } else {
        max_bin as f64 * bin_hz
    };

    let centroid = if sum_mag > 1e-12 {
        sum_freq_mag / sum_mag
    } else {
        0.0
    };
    (dom_hz, centroid)
}

fn fft_inplace(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
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
    let mut len = 2usize;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let wlen_re = ang.cos();
        let wlen_im = ang.sin();
        let half = len / 2;
        for i in (0..n).step_by(len) {
            let mut w_re = 1.0_f64;
            let mut w_im = 0.0_f64;
            for k in 0..half {
                let u_re = re[i + k];
                let u_im = im[i + k];
                let v_re = re[i + k + half] * w_re - im[i + k + half] * w_im;
                let v_im = re[i + k + half] * w_im + im[i + k + half] * w_re;
                re[i + k] = u_re + v_re;
                im[i + k] = u_im + v_im;
                re[i + k + half] = u_re - v_re;
                im[i + k + half] = u_im - v_im;
                let next_w_re = w_re * wlen_re - w_im * wlen_im;
                let next_w_im = w_re * wlen_im + w_im * wlen_re;
                w_re = next_w_re;
                w_im = next_w_im;
            }
        }
        len <<= 1;
    }
}

pub fn l2_diff_rms(a_l: &[f32], a_r: &[f32], b_l: &[f32], b_r: &[f32]) -> f64 {
    let n = a_l.len().min(a_r.len()).min(b_l.len()).min(b_r.len());
    if n == 0 {
        return 0.0;
    }
    let mut acc = 0.0_f64;
    for i in 0..n {
        let dl = (a_l[i] - b_l[i]) as f64;
        let dr = (a_r[i] - b_r[i]) as f64;
        acc += 0.5 * (dl * dl + dr * dr);
    }
    (acc / n as f64).sqrt()
}

pub fn run_dry_run_self_tests() -> Vec<(String, String, bool, String)> {
    let mut results = Vec::new();
    let sr = 48000.0;
    let n = 8192;

    // 1. Clean 440 Hz sine wave -> MUST pass BASI and detect ~440 Hz
    let mut sine = vec![0.0_f32; n];
    for i in 0..n {
        sine[i] = (2.0 * PI * 440.0 * (i as f64) / sr).sin() as f32 * 0.5;
    }
    let m_sine = analyze_stereo(&sine, &sine, sr);
    let ok_sine = m_sine.basi_pass
        && (m_sine.dominant_freq_hz - 440.0).abs() < 3.0
        && m_sine.rms_dbfs > -12.0;
    results.push((
        "DRY-01-CLEAN-SINE".into(),
        "Analyzer validates clean 440 Hz sine (BASI + FFT pitch accuracy)".into(),
        ok_sine,
        format!("basi={} dom_hz={:.2} rms_dbfs={:.2}", m_sine.basi_pass, m_sine.dominant_freq_hz, m_sine.rms_dbfs),
    ));

    // 2. Negative test: NaN injection MUST fail BASI
    let mut bad_nan = sine.clone();
    bad_nan[100] = f32::NAN;
    let m_nan = analyze_stereo(&bad_nan, &sine, sr);
    results.push((
        "DRY-02-REJECT-NAN".into(),
        "Analyzer rejects NaN audio samples".into(),
        !m_nan.basi_pass && m_nan.nan_count == 1,
        format!("basi={} nan_count={}", m_nan.basi_pass, m_nan.nan_count),
    ));

    // 3. Negative test: Clipping peak > 0.995 MUST fail BASI
    let mut bad_clip = sine.clone();
    bad_clip[500] = 1.05;
    let m_clip = analyze_stereo(&bad_clip, &sine, sr);
    results.push((
        "DRY-03-REJECT-CLIP".into(),
        "Analyzer rejects clipping peak > 0.995".into(),
        !m_clip.basi_pass && m_clip.peak_max > 0.995,
        format!("basi={} peak={:.3}", m_clip.basi_pass, m_clip.peak_max),
    ));

    // 4. Negative test: DC offset > 0.005 MUST fail BASI
    let bad_dc: Vec<f32> = sine.iter().map(|s| s + 0.05).collect();
    let m_dc = analyze_stereo(&bad_dc, &bad_dc, sr);
    results.push((
        "DRY-04-REJECT-DC".into(),
        "Analyzer rejects DC offset > 0.005".into(),
        !m_dc.basi_pass && m_dc.dc_max_abs > 0.005,
        format!("basi={} dc={:.4}", m_dc.basi_pass, m_dc.dc_max_abs),
    ));

    // 5. L2 diff between 440 Hz and 880 Hz MUST be significant
    let mut sine880 = vec![0.0_f32; n];
    for i in 0..n {
        sine880[i] = (2.0 * PI * 880.0 * (i as f64) / sr).sin() as f32 * 0.5;
    }
    let diff = l2_diff_rms(&sine, &sine, &sine880, &sine880);
    results.push((
        "DRY-05-L2-DIFF".into(),
        "L2 waveform diff distinguishes distinct signals".into(),
        diff > 0.3,
        format!("l2_diff_rms={:.4}", diff),
    ));

    results
}
