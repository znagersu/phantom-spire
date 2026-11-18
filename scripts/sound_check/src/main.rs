use std::cell::RefCell;
use std::ffi::{c_void, CStr, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use vst3_sys::base::{
    kInvalidArgument, kResultFalse, kResultOk, tresult, IPluginBase, IPluginFactory, PClassInfo,
    TBool,
};
use vst3_sys::utils::{StaticVstPtr, VstPtr};
use vst3_sys::vst::{
    AudioBusBuffers, BusDirections, Event, EventData, EventTypes, IAudioProcessor, IComponent,
    IEventList, IHostApplication, IParamValueQueue, IParameterChanges, MediaTypes, NoteOffEvent,
    NoteOnEvent, ProcessContext, ProcessData, ProcessModes, ProcessSetup, SymbolicSampleSizes,
};
use vst3_sys::{ComInterface, IID, VST3};
use winapi::shared::minwindef::HMODULE;
use winapi::um::libloaderapi::{FreeLibrary, GetProcAddress, LoadLibraryW};

// ============================================================================
// Minimal COM Host Glue for Headless VST3 Rendering
// ============================================================================

#[VST3(implements(IHostApplication))]
struct HostApp {}

impl HostApp {
    fn new() -> Box<Self> {
        Self::allocate()
    }
}

impl IHostApplication for HostApp {
    unsafe fn get_name(&self, name: *mut u16) -> tresult {
        if !name.is_null() {
            let s = "PhantomSpireSoundCheck";
            for (i, ch) in s.encode_utf16().take(127).enumerate() {
                *name.add(i) = ch;
            }
            *name.add(s.len().min(127)) = 0;
        }
        kResultOk
    }

    unsafe fn create_instance(
        &self,
        _cid: *const IID,
        _iid: *const IID,
        obj: *mut *mut c_void,
    ) -> tresult {
        if !obj.is_null() {
            *obj = ptr::null_mut();
        }
        kResultFalse
    }
}

#[VST3(implements(IEventList))]
struct HostEventList {
    events: RefCell<Vec<Event>>,
}

impl HostEventList {
    fn new(events: Vec<Event>) -> Box<Self> {
        Self::allocate(RefCell::new(events))
    }
}

impl IEventList for HostEventList {
    unsafe fn get_event_count(&self) -> i32 {
        self.events.borrow().len() as i32
    }

    unsafe fn get_event(&self, index: i32, e: *mut Event) -> tresult {
        let evs = self.events.borrow();
        if index < 0 || (index as usize) >= evs.len() || e.is_null() {
            return kInvalidArgument;
        }
        *e = evs[index as usize];
        kResultOk
    }

    unsafe fn add_event(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return kInvalidArgument;
        }
        self.events.borrow_mut().push(*e);
        kResultOk
    }
}

#[VST3(implements(IParamValueQueue))]
struct HostParamValueQueue {
    param_id: u32,
}

impl IParamValueQueue for HostParamValueQueue {
    unsafe fn get_parameter_id(&self) -> u32 {
        self.param_id
    }
    unsafe fn get_point_count(&self) -> i32 {
        0
    }
    unsafe fn get_point(&self, _index: i32, _offset: *mut i32, _val: *mut f64) -> tresult {
        kInvalidArgument
    }
    unsafe fn add_point(&self, _offset: i32, _val: f64, _index: *mut i32) -> tresult {
        kInvalidArgument
    }
}

#[VST3(implements(IParameterChanges))]
struct EmptyParamChanges {}

impl EmptyParamChanges {
    fn new() -> Box<Self> {
        Self::allocate()
    }
}

impl IParameterChanges for EmptyParamChanges {
    unsafe fn get_parameter_count(&self) -> i32 {
        0
    }
    unsafe fn get_parameter_data(&self, _index: i32) -> StaticVstPtr<dyn IParamValueQueue> {
        std::mem::transmute(ptr::null_mut::<c_void>())
    }
    unsafe fn add_parameter_data(
        &self,
        _id: *const u32,
        _index: *mut i32,
    ) -> StaticVstPtr<dyn IParamValueQueue> {
        std::mem::transmute(ptr::null_mut::<c_void>())
    }
}

struct Vst3Host {
    component: Option<VstPtr<dyn IComponent>>,
    processor: Option<VstPtr<dyn IAudioProcessor>>,
    _host_app: Box<HostApp>,
    exit_dll: Option<unsafe extern "system" fn() -> bool>,
    module: HMODULE,
    sample_rate: f64,
    block_size: usize,
    sample_pos: i64,
}

fn resolve_vst3_dll(bundle_or_dll: &Path) -> PathBuf {
    if bundle_or_dll.is_dir() {
        let leaf = bundle_or_dll
            .file_name()
            .unwrap_or_else(|| OsStr::new("phantom_spire.vst3"));
        let candidate = bundle_or_dll.join("Contents").join("x86_64-win").join(leaf);
        if candidate.is_file() {
            return candidate;
        }
    }
    bundle_or_dll.to_path_buf()
}

fn cstr_from_i8(buf: &[i8]) -> String {
    unsafe { CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned() }
}

impl Vst3Host {
    fn load(bundle_path: &Path, sample_rate: f64, block_size: usize) -> Result<Self, String> {
        let dll = resolve_vst3_dll(bundle_path);
        if !dll.is_file() {
            return Err(format!("VST3 binary not found: {}", dll.display()));
        }
        let wide: Vec<u16> = dll.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

        unsafe {
            let module = LoadLibraryW(wide.as_ptr());
            if module.is_null() {
                return Err(format!("LoadLibraryW failed for {}", dll.display()));
            }
            let init_sym = GetProcAddress(module, b"InitDll\0".as_ptr() as *const i8);
            if !init_sym.is_null() {
                let init_fn: unsafe extern "system" fn() -> bool = std::mem::transmute(init_sym);
                let _ = init_fn();
            }
            let exit_sym = GetProcAddress(module, b"ExitDll\0".as_ptr() as *const i8);
            let exit_dll = if !exit_sym.is_null() {
                Some(std::mem::transmute(exit_sym))
            } else {
                None
            };

            let factory_sym = GetProcAddress(module, b"GetPluginFactory\0".as_ptr() as *const i8);
            if factory_sym.is_null() {
                FreeLibrary(module);
                return Err("GetPluginFactory export missing".into());
            }
            let get_factory: unsafe extern "system" fn() -> *mut *mut <dyn IPluginFactory as ComInterface>::VTable =
                std::mem::transmute(factory_sym);
            let factory = VstPtr::<dyn IPluginFactory>::owned(get_factory())
                .ok_or_else(|| "GetPluginFactory returned null".to_string())?;

            let mut audio_cid: Option<IID> = None;
            for i in 0..factory.count_classes() {
                let mut cinfo: PClassInfo = std::mem::zeroed();
                if factory.get_class_info(i, &mut cinfo) == kResultOk
                    && cstr_from_i8(&cinfo.category) == "Audio Module Class"
                {
                    audio_cid = Some(cinfo.cid);
                    break;
                }
            }
            let cid = audio_cid.ok_or_else(|| "No Audio Module Class found".to_string())?;

            let mut comp_raw: *mut c_void = ptr::null_mut();
            if factory.create_instance(&cid, &<dyn IComponent as ComInterface>::IID, &mut comp_raw)
                != kResultOk
                || comp_raw.is_null()
            {
                return Err("create_instance(IComponent) failed".into());
            }
            let component = VstPtr::<dyn IComponent>::owned(comp_raw as *mut _)
                .ok_or_else(|| "Invalid IComponent ptr".to_string())?;

            let host_app = HostApp::new();
            let host_ctx: *const HostApp = &*host_app;
            if component.initialize(host_ctx as *mut c_void) != kResultOk {
                return Err("IComponent::initialize failed".into());
            }
            let processor = component
                .cast::<dyn IAudioProcessor>()
                .ok_or_else(|| "Missing IAudioProcessor".to_string())?;

            let _ = component.activate_bus(
                MediaTypes::kAudio as i32,
                BusDirections::kOutput as i32,
                0,
                1 as TBool,
            );
            let _ = component.activate_bus(
                MediaTypes::kEvent as i32,
                BusDirections::kInput as i32,
                0,
                1 as TBool,
            );

            let setup = ProcessSetup {
                process_mode: ProcessModes::kRealtime as i32,
                symbolic_sample_size: SymbolicSampleSizes::kSample32 as i32,
                max_samples_per_block: block_size as i32,
                sample_rate,
            };
            if processor.setup_processing(&setup) != kResultOk {
                return Err("setup_processing failed".into());
            }
            if component.set_active(1) != kResultOk {
                return Err("set_active(1) failed".into());
            }
            let p_res = processor.set_processing(1);
            if p_res != kResultOk && p_res != kResultFalse {
                return Err("set_processing(1) failed".into());
            }

            Ok(Self {
                component: Some(component),
                processor: Some(processor),
                _host_app: host_app,
                exit_dll,
                module,
                sample_rate,
                block_size,
                sample_pos: 0,
            })
        }
    }

    fn process_block(
        &mut self,
        events: Vec<Event>,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) -> Result<(), String> {
        let n = out_l.len().min(out_r.len());
        out_l[..n].fill(0.0);
        out_r[..n].fill(0.0);

        unsafe {
            let ev_list = HostEventList::new(events);
            let in_pc = EmptyParamChanges::new();
            let out_pc = EmptyParamChanges::new();
            let out_ev = HostEventList::new(Vec::new());

            let mut ch_ptrs: [*mut c_void; 2] = [
                out_l.as_mut_ptr() as *mut c_void,
                out_r.as_mut_ptr() as *mut c_void,
            ];
            let mut out_bus = AudioBusBuffers {
                num_channels: 2,
                silence_flags: 0,
                buffers: ch_ptrs.as_mut_ptr(),
            };

            let mut ctx: ProcessContext = std::mem::zeroed();
            ctx.state = 1 | (1 << 9) | (1 << 10);
            ctx.sample_rate = self.sample_rate;
            ctx.project_time_samples = self.sample_pos;
            ctx.project_time_music = (self.sample_pos as f64 / self.sample_rate) * 2.0;
            ctx.tempo = 120.0;
            ctx.time_sig_num = 4;
            ctx.time_sig_den = 4;

            let ev_ptr: *const HostEventList = &*ev_list;
            let in_pc_ptr: *const EmptyParamChanges = &*in_pc;
            let out_pc_ptr: *const EmptyParamChanges = &*out_pc;
            let out_ev_ptr: *const HostEventList = &*out_ev;

            let mut data = ProcessData {
                process_mode: ProcessModes::kRealtime as i32,
                symbolic_sample_size: SymbolicSampleSizes::kSample32 as i32,
                num_samples: n as i32,
                num_inputs: 0,
                num_outputs: 1,
                inputs: ptr::null_mut(),
                outputs: &mut out_bus,
                input_param_changes: std::mem::transmute(in_pc_ptr),
                output_param_changes: std::mem::transmute(out_pc_ptr),
                input_events: std::mem::transmute(ev_ptr),
                output_events: std::mem::transmute(out_ev_ptr),
                context: &mut ctx,
            };

            let processor = self.processor.as_ref().ok_or("processor missing")?;
            let res = processor.process(&mut data);
            self.sample_pos += n as i64;
            if res != kResultOk {
                return Err(format!("IAudioProcessor::process failed: {res}"));
            }
            Ok(())
        }
    }

    fn note_event(&self, is_on: bool, offset: i32, pitch: i16, velocity: f32) -> Event {
        let ppq = (self.sample_pos as f64 / self.sample_rate) * 2.0;
        if is_on {
            Event {
                bus_index: 0,
                sample_offset: offset,
                ppq_position: ppq,
                flags: 0,
                type_: EventTypes::kNoteOnEvent as u16,
                event: EventData {
                    note_on: NoteOnEvent {
                        channel: 0,
                        pitch,
                        tuning: 0.0,
                        velocity,
                        length: 0,
                        note_id: -1,
                    },
                },
            }
        } else {
            Event {
                bus_index: 0,
                sample_offset: offset,
                ppq_position: ppq,
                flags: 0,
                type_: EventTypes::kNoteOffEvent as u16,
                event: EventData {
                    note_off: NoteOffEvent {
                        channel: 0,
                        pitch,
                        velocity: 0.0,
                        note_id: -1,
                        tuning: 0.0,
                    },
                },
            }
        }
    }
}

impl Drop for Vst3Host {
    fn drop(&mut self) {
        unsafe {
            if let Some(ref p) = self.processor {
                let _ = p.set_processing(0);
            }
            if let Some(ref c) = self.component {
                let _ = c.set_active(0);
                let _ = c.terminate();
            }
            self.processor.take();
            self.component.take();
            if let Some(exit_fn) = self.exit_dll {
                let _ = exit_fn();
            }
            FreeLibrary(self.module);
        }
    }
}

// ============================================================================
// WAV Writing & Audio Quality Analysis ("Ears")
// ============================================================================

fn write_wav_pcm16(path: &Path, sample_rate: u32, left: &[f32], right: &[f32]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create wav parent '{}': {e}", parent.display()))?;
    }
    let frames = left.len().min(right.len());
    let data_bytes = (frames * 4) as u32;
    let riff_size = 36u32 + data_bytes;

    let mut f = File::create(path)
        .map_err(|e| format!("Failed to create wav '{}': {e}", path.display()))?;
    f.write_all(b"RIFF").map_err(|e| e.to_string())?;
    f.write_all(&riff_size.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(b"WAVE").map_err(|e| e.to_string())?;
    f.write_all(b"fmt ").map_err(|e| e.to_string())?;
    f.write_all(&16u32.to_le_bytes()).map_err(|e| e.to_string())?; // PCM fmt chunk size
    f.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?; // PCM = 1
    f.write_all(&2u16.to_le_bytes()).map_err(|e| e.to_string())?; // stereo
    f.write_all(&sample_rate.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(&(sample_rate * 4).to_le_bytes()).map_err(|e| e.to_string())?; // byte rate
    f.write_all(&4u16.to_le_bytes()).map_err(|e| e.to_string())?; // block align
    f.write_all(&16u16.to_le_bytes()).map_err(|e| e.to_string())?; // bits per sample
    f.write_all(b"data").map_err(|e| e.to_string())?;
    f.write_all(&data_bytes.to_le_bytes()).map_err(|e| e.to_string())?;

    let mut buf = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let l_q = (left[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let r_q = (right[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
        buf.extend_from_slice(&l_q.to_le_bytes());
        buf.extend_from_slice(&r_q.to_le_bytes());
    }
    f.write_all(&buf).map_err(|e| e.to_string())?;
    Ok(())
}

fn read_wav_pcm16(path: &Path) -> Result<(u32, Vec<f32>, Vec<f32>), String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("Failed to open wav '{}': {e}", path.display()))?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;

    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Invalid WAV header".into());
    }
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
    if channels != 2 || bits != 16 {
        return Err(format!("Unexpected WAV format: {channels}ch {bits}bit"));
    }

    let data = &bytes[44..];
    let frames = data.len() / 4;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for i in 0..frames {
        let off = i * 4;
        let l = i16::from_le_bytes([data[off], data[off + 1]]) as f32 / 32767.0;
        let r = i16::from_le_bytes([data[off + 2], data[off + 3]]) as f32 / 32767.0;
        left.push(l);
        right.push(r);
    }
    Ok((sample_rate, left, right))
}

fn window_rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

fn max_sample_jump(samples: &[f32], center: usize, radius: usize) -> f32 {
    let start = center.saturating_sub(radius).max(1);
    let end = (center + radius).min(samples.len());
    let mut max_j = 0.0_f32;
    for i in start..end {
        max_j = max_j.max((samples[i] - samples[i - 1]).abs());
    }
    max_j
}

fn analyze_audio_ears(
    float_l: &[f32],
    float_r: &[f32],
    wav_path: &Path,
    note_off_samples: &[usize],
    active_ranges: &[(usize, usize)],
) -> Result<(), String> {
    let mut issues = Vec::new();

    // 1. Raw float safety (NaN / Inf / subnormals / hard float clipping)
    let mut non_finite = 0usize;
    let mut float_peak = 0.0_f32;
    for &s in float_l.iter().chain(float_r.iter()) {
        if !s.is_finite() {
            non_finite += 1;
        } else {
            float_peak = float_peak.max(s.abs());
        }
    }
    if non_finite > 0 {
        issues.push(format!("Non-finite samples detected (NaN/Inf): {non_finite}"));
    }
    if float_peak >= 0.998 {
        issues.push(format!("Audio clipping detected in float output (peak={float_peak:.4} >= 0.998)"));
    }

    // 2. Read back the saved .wav file from disk and inspect PCM audio
    let (sr, wav_l, wav_r) = read_wav_pcm16(wav_path)?;
    if wav_l.is_empty() {
        return Err("Saved WAV has zero frames".into());
    }

    // Check DC offset across full file
    let dc_l = wav_l.iter().map(|&s| s as f64).sum::<f64>() / wav_l.len() as f64;
    let dc_r = wav_r.iter().map(|&s| s as f64).sum::<f64>() / wav_r.len() as f64;
    if dc_l.abs() > 0.01 || dc_r.abs() > 0.01 {
        issues.push(format!("Excessive DC offset (L={dc_l:.5}, R={dc_r:.5})"));
    }

    // Check consecutive full-scale clipped samples in WAV
    let mut max_clip_run = 0usize;
    let mut cur_run = 0usize;
    for &s in wav_l.iter().chain(wav_r.iter()) {
        if s.abs() >= 0.998 {
            cur_run += 1;
            max_clip_run = max_clip_run.max(cur_run);
        } else {
            cur_run = 0;
        }
    }
    if max_clip_run >= 3 {
        issues.push(format!("Sustained WAV clipping run detected ({max_clip_run} samples)"));
    }

    // Check Dead Silence & Harsh Static in active note/chord windows
    for (idx, &(start, end)) in active_ranges.iter().enumerate() {
        let s = start.min(wav_l.len());
        let e = end.min(wav_l.len());
        let rms_l = window_rms(&wav_l[s..e]);
        let rms_r = window_rms(&wav_r[s..e]);
        let rms = rms_l.max(rms_r);
        let rms_db = if rms > 1e-9 { 20.0 * rms.log10() } else { -180.0 };
        if rms_db < -45.0 {
            issues.push(format!("Dead silence in active segment #{idx} (RMS={rms_db:.1} dBFS)"));
        }

        // Sample-to-sample derivative ratio flags white-noise / aliasing static
        if e > s + 2 {
            let diff_sq: f64 = (s + 1..e)
                .map(|i| {
                    let dl = (wav_l[i] - wav_l[i - 1]) as f64;
                    let dr = (wav_r[i] - wav_r[i - 1]) as f64;
                    0.5 * (dl * dl + dr * dr)
                })
                .sum();
            let diff_rms = (diff_sq / (e - s - 1) as f64).sqrt();
            let avg_rms = (0.5 * (rms_l * rms_l + rms_r * rms_r)).sqrt().max(1e-6);
            let static_ratio = diff_rms / avg_rms;
            if static_ratio > 0.75 {
                issues.push(format!(
                    "Harsh static / excessive high-freq noise in segment #{idx} (diff/rms={static_ratio:.3} > 0.75)"
                ));
            }
        }
    }

    // Check Loud Clicks when notes cut off (±5ms around NoteOff boundaries)
    let radius = ((sr as usize) * 5) / 1000;
    let mut worst_cutoff_jump = 0.0_f32;
    for &off_idx in note_off_samples {
        let jl = max_sample_jump(&wav_l, off_idx, radius);
        let jr = max_sample_jump(&wav_r, off_idx, radius);
        let j = jl.max(jr);
        worst_cutoff_jump = worst_cutoff_jump.max(j);
        if j >= 0.25 {
            issues.push(format!(
                "Loud click/pop on note cut-off near sample {off_idx} (step jump={j:.4} >= 0.25)"
            ));
        }
    }

    println!(
        "[sound_check] WAV={} sr={}Hz frames={} peak={:.4} dc=({:.5},{:.5}) worst_note_off_jump={:.4}",
        wav_path.display(),
        sr,
        wav_l.len(),
        float_peak,
        dc_l,
        dc_r,
        worst_cutoff_jump
    );

    if issues.is_empty() {
        println!("[sound_check] PASS: No dead silence, no clipping, no harsh static, no note-off clicks.");
        Ok(())
    } else {
        for err in &issues {
            eprintln!("[sound_check] FAIL: {err}");
        }
        Err(issues.join("; "))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut plugin_path = PathBuf::from("target/bundled/phantom_spire.vst3");
    let mut wav_path = std::env::temp_dir()
        .join("phantom_spire_soundcheck")
        .join("soundcheck.wav");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--plugin" if i + 1 < args.len() => {
                plugin_path = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--wav" if i + 1 < args.len() => {
                wav_path = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let sr = 44100.0;
    let bs = 512usize;
    let mut host = match Vst3Host::load(&plugin_path, sr, bs) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[sound_check] ERROR loading plugin: {e}");
            std::process::exit(1);
        }
    };

    // Timeline (512-sample blocks @ 44.1kHz):
    //   0..35   (0.00s..0.41s): Single note C4 (60) ON
    //  35..60   (0.41s..0.70s): Note C4 OFF (release tail)
    //  60..120  (0.70s..1.39s): 4-voice Cmaj7 chord (60, 64, 67, 71) ON
    // 120..165  (1.39s..1.92s): Chord OFF (release tail)
    let total_blocks = 165usize;
    let mut out_l = Vec::with_capacity(total_blocks * bs);
    let mut out_r = Vec::with_capacity(total_blocks * bs);
    let mut blk_l = vec![0.0_f32; bs];
    let mut blk_r = vec![0.0_f32; bs];

    let chord = [60i16, 64, 67, 71];
    for b in 0..total_blocks {
        let mut evs = Vec::new();
        if b == 0 {
            evs.push(host.note_event(true, 0, 60, 0.85));
        } else if b == 35 {
            evs.push(host.note_event(false, 0, 60, 0.0));
        } else if b == 60 {
            for &n in &chord {
                evs.push(host.note_event(true, 0, n, 0.72));
            }
        } else if b == 120 {
            for &n in &chord {
                evs.push(host.note_event(false, 0, n, 0.0));
            }
        }
        if let Err(e) = host.process_block(evs, &mut blk_l, &mut blk_r) {
            eprintln!("[sound_check] ERROR processing block {b}: {e}");
            std::process::exit(1);
        }
        out_l.extend_from_slice(&blk_l);
        out_r.extend_from_slice(&blk_r);
    }

    if let Err(e) = write_wav_pcm16(&wav_path, sr as u32, &out_l, &out_r) {
        eprintln!("[sound_check] ERROR writing WAV: {e}");
        std::process::exit(1);
    }

    let note_off_samples = [35 * bs, 120 * bs];
    let active_ranges = [(bs, 34 * bs), (61 * bs, 119 * bs)];
    if analyze_audio_ears(&out_l, &out_r, &wav_path, &note_off_samples, &active_ranges).is_err() {
        std::process::exit(1);
    }
}
