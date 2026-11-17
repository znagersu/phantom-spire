use std::cell::RefCell;
use std::ffi::{c_void, CStr, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use vst3_sys::base::{
    kIBSeekCur, kIBSeekEnd, kIBSeekSet, kInvalidArgument, kResultFalse, kResultOk, tresult,
    IBStream, IPluginBase, IPluginFactory, PClassInfo, PFactoryInfo, TBool,
};
use vst3_sys::utils::{SharedVstPtr, StaticVstPtr, VstPtr};
use vst3_sys::vst::{
    AudioBusBuffers, BusDirections, Event, EventData, EventTypes, IAudioProcessor, IComponent,
    IEditController, IEventList, IHostApplication, IParamValueQueue, IParameterChanges, MediaTypes,
    NoteOffEvent, NoteOnEvent, ParameterInfo, ProcessContext, ProcessData, ProcessModes,
    ProcessSetup, SymbolicSampleSizes,
};
use vst3_sys::{ComInterface, IID, VST3};
use winapi::shared::minwindef::HMODULE;
use winapi::um::libloaderapi::{FreeLibrary, GetProcAddress, LoadLibraryW};

// ============================================================================
// Minimal Host COM Objects (IHostApplication, IEventList, IParameterChanges, IBStream)
// ============================================================================

#[VST3(implements(IHostApplication))]
pub struct HostApp {}

impl HostApp {
    pub fn new() -> Box<Self> {
        Self::allocate()
    }
}

impl IHostApplication for HostApp {
    unsafe fn get_name(&self, name: *mut u16) -> tresult {
        if !name.is_null() {
            let s = "PhantomSpireE2EHost";
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
pub struct HostEventList {
    events: RefCell<Vec<Event>>,
}

impl HostEventList {
    pub fn new(events: Vec<Event>) -> Box<Self> {
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
pub struct HostParamValueQueue {
    param_id: u32,
    points: RefCell<Vec<(i32, f64)>>,
}

impl HostParamValueQueue {
    pub fn new(param_id: u32, points: Vec<(i32, f64)>) -> Box<Self> {
        Self::allocate(param_id, RefCell::new(points))
    }
}

impl IParamValueQueue for HostParamValueQueue {
    unsafe fn get_parameter_id(&self) -> u32 {
        self.param_id
    }

    unsafe fn get_point_count(&self) -> i32 {
        self.points.borrow().len() as i32
    }

    unsafe fn get_point(&self, index: i32, sample_offset: *mut i32, value: *mut f64) -> tresult {
        let pts = self.points.borrow();
        if index < 0 || (index as usize) >= pts.len() {
            return kInvalidArgument;
        }
        let (off, val) = pts[index as usize];
        if !sample_offset.is_null() {
            *sample_offset = off;
        }
        if !value.is_null() {
            *value = val;
        }
        kResultOk
    }

    unsafe fn add_point(&self, sample_offset: i32, value: f64, index: *mut i32) -> tresult {
        let mut pts = self.points.borrow_mut();
        pts.push((sample_offset, value));
        if !index.is_null() {
            *index = (pts.len() - 1) as i32;
        }
        kResultOk
    }
}

#[VST3(implements(IParameterChanges))]
pub struct HostParamChanges {
    queues: RefCell<Vec<Box<HostParamValueQueue>>>,
}

impl HostParamChanges {
    pub fn new(changes: Vec<(u32, f64)>) -> Box<Self> {
        let queues = changes
            .into_iter()
            .map(|(id, val)| HostParamValueQueue::new(id, vec![(0, val)]))
            .collect();
        Self::allocate(RefCell::new(queues))
    }
}

impl IParameterChanges for HostParamChanges {
    unsafe fn get_parameter_count(&self) -> i32 {
        self.queues.borrow().len() as i32
    }

    unsafe fn get_parameter_data(&self, index: i32) -> StaticVstPtr<dyn IParamValueQueue> {
        let qs = self.queues.borrow();
        if index < 0 || (index as usize) >= qs.len() {
            return std::mem::transmute(ptr::null_mut::<c_void>());
        }
        let raw_ptr: *const HostParamValueQueue = &*qs[index as usize];
        std::mem::transmute(raw_ptr)
    }

    unsafe fn add_parameter_data(
        &self,
        id: *const u32,
        index: *mut i32,
    ) -> StaticVstPtr<dyn IParamValueQueue> {
        let pid = if id.is_null() { 0 } else { *id };
        let mut qs = self.queues.borrow_mut();
        qs.push(HostParamValueQueue::new(pid, Vec::new()));
        let idx = qs.len() - 1;
        if !index.is_null() {
            *index = idx as i32;
        }
        let raw_ptr: *const HostParamValueQueue = &*qs[idx];
        std::mem::transmute(raw_ptr)
    }
}

#[VST3(implements(IBStream))]
pub struct HostMemStream {
    data: RefCell<Vec<u8>>,
    pos: RefCell<usize>,
}

impl HostMemStream {
    pub fn new(initial: Vec<u8>) -> Box<Self> {
        Self::allocate(RefCell::new(initial), RefCell::new(0))
    }

    pub fn take_bytes(&self) -> Vec<u8> {
        self.data.borrow().clone()
    }
}

impl IBStream for HostMemStream {
    unsafe fn read(
        &self,
        buffer: *mut c_void,
        num_bytes: i32,
        num_bytes_read: *mut i32,
    ) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let data = self.data.borrow();
        let mut pos = self.pos.borrow_mut();
        let avail = data.len().saturating_sub(*pos);
        let to_read = (num_bytes as usize).min(avail);
        if to_read > 0 {
            ptr::copy_nonoverlapping(data.as_ptr().add(*pos), buffer as *mut u8, to_read);
            *pos += to_read;
        }
        if !num_bytes_read.is_null() {
            *num_bytes_read = to_read as i32;
        }
        kResultOk
    }

    unsafe fn write(
        &self,
        buffer: *const c_void,
        num_bytes: i32,
        num_bytes_written: *mut i32,
    ) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let mut data = self.data.borrow_mut();
        let mut pos = self.pos.borrow_mut();
        let n = num_bytes as usize;
        let slice = std::slice::from_raw_parts(buffer as *const u8, n);
        if *pos + n > data.len() {
            data.resize(*pos + n, 0);
        }
        data[*pos..*pos + n].copy_from_slice(slice);
        *pos += n;
        if !num_bytes_written.is_null() {
            *num_bytes_written = n as i32;
        }
        kResultOk
    }

    unsafe fn seek(&self, offset: i64, mode: i32, result: *mut i64) -> tresult {
        let len = self.data.borrow().len() as i64;
        let mut pos = self.pos.borrow_mut();
        let new_pos = match mode {
            x if x == kIBSeekSet => offset,
            x if x == kIBSeekCur => *pos as i64 + offset,
            x if x == kIBSeekEnd => len + offset,
            _ => return kInvalidArgument,
        };
        if new_pos < 0 {
            return kInvalidArgument;
        }
        *pos = new_pos as usize;
        if !result.is_null() {
            *result = new_pos;
        }
        kResultOk
    }

    unsafe fn tell(&self, pos_out: *mut i64) -> tresult {
        if !pos_out.is_null() {
            *pos_out = *self.pos.borrow() as i64;
        }
        kResultOk
    }
}

// ============================================================================
// High-Level Opaque-Box VST3 Plugin Instance Wrapper
// ============================================================================

#[derive(Debug, Clone)]
pub struct Vst3ParamMeta {
    pub id: u32,
    pub title: String,
    pub short_title: String,
    pub default_normalized: f64,
    pub step_count: i32,
}

#[derive(Debug, Clone, Copy)]
pub enum MidiEvent {
    NoteOn { sample_offset: i32, note: i16, velocity: f32 },
    NoteOff { sample_offset: i32, note: i16 },
}

pub struct Vst3PluginInstance {
    component: Option<VstPtr<dyn IComponent>>,
    processor: Option<VstPtr<dyn IAudioProcessor>>,
    controller: Option<VstPtr<dyn IEditController>>,
    _host_app: Box<HostApp>,
    exit_dll: Option<unsafe extern "system" fn() -> bool>,
    module: HMODULE,
    pub vendor: String,
    pub plugin_name: String,
    pub params: Vec<Vst3ParamMeta>,
    sample_rate: f64,
    block_size: usize,
    sample_pos: i64,
    bpm: f64,
}

fn resolve_vst3_dll_path(bundle_or_dll: &Path) -> std::path::PathBuf {
    if bundle_or_dll.is_dir() {
        let candidate = bundle_or_dll
            .join("Contents")
            .join("x86_64-win")
            .join(
                bundle_or_dll
                    .file_name()
                    .unwrap_or_else(|| OsStr::new("phantom_spire.vst3")),
            );
        if candidate.is_file() {
            return candidate;
        }
    }
    bundle_or_dll.to_path_buf()
}

fn cstr_from_i8(buf: &[i8]) -> String {
    unsafe {
        CStr::from_ptr(buf.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

fn utf16_from_i16(buf: &[i16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let u16s: Vec<u16> = buf[..end].iter().map(|&c| c as u16).collect();
    String::from_utf16_lossy(&u16s)
}

impl Vst3PluginInstance {
    pub fn load(bundle_path: &Path, sample_rate: f64, block_size: usize) -> Result<Self, String> {
        let dll_path = resolve_vst3_dll_path(bundle_path);
        if !dll_path.is_file() {
            return Err(format!("VST3 DLL not found at {}", dll_path.display()));
        }

        let wide: Vec<u16> = dll_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let module = LoadLibraryW(wide.as_ptr());
            if module.is_null() {
                return Err(format!("LoadLibraryW failed for {}", dll_path.display()));
            }

            let init_sym = GetProcAddress(module, b"InitDll\0".as_ptr() as *const i8);
            if !init_sym.is_null() {
                let init_fn: unsafe extern "system" fn() -> bool = std::mem::transmute(init_sym);
                let _ = init_fn();
            }

            let exit_sym = GetProcAddress(module, b"ExitDll\0".as_ptr() as *const i8);
            let exit_dll: Option<unsafe extern "system" fn() -> bool> = if !exit_sym.is_null() {
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
            let raw_factory = get_factory();
            let factory = VstPtr::<dyn IPluginFactory>::owned(raw_factory)
                .ok_or_else(|| "GetPluginFactory returned null".to_string())?;

            let mut finfo: PFactoryInfo = std::mem::zeroed();
            factory.get_factory_info(&mut finfo);
            let vendor = cstr_from_i8(&finfo.vendor);

            let class_count = factory.count_classes();
            let mut audio_cid: Option<IID> = None;
            let mut plugin_name = String::new();
            for i in 0..class_count {
                let mut cinfo: PClassInfo = std::mem::zeroed();
                if factory.get_class_info(i, &mut cinfo) == kResultOk {
                    let cat = cstr_from_i8(&cinfo.category);
                    if cat == "Audio Module Class" {
                        audio_cid = Some(cinfo.cid);
                        plugin_name = cstr_from_i8(&cinfo.name);
                        break;
                    }
                }
            }
            let cid = audio_cid.ok_or_else(|| "No Audio Module Class found in VST3 factory".to_string())?;

            let mut comp_raw: *mut c_void = ptr::null_mut();
            let res = factory.create_instance(
                &cid,
                &<dyn IComponent as ComInterface>::IID,
                &mut comp_raw,
            );
            if res != kResultOk || comp_raw.is_null() {
                return Err(format!("IPluginFactory::create_instance(IComponent) failed: {}", res));
            }
            let component = VstPtr::<dyn IComponent>::owned(comp_raw as *mut _)
                .ok_or_else(|| "IComponent VstPtr creation failed".to_string())?;

            let host_app = HostApp::new();
            let host_ctx: *const HostApp = &*host_app;
            let init_res = component.initialize(host_ctx as *mut c_void);
            if init_res != kResultOk {
                return Err(format!("IComponent::initialize failed: {}", init_res));
            }

            let processor = component
                .cast::<dyn IAudioProcessor>()
                .ok_or_else(|| "Plugin does not implement IAudioProcessor".to_string())?;

            let controller = component.cast::<dyn IEditController>();
            let mut params = Vec::new();
            if let Some(ref ctrl) = controller {
                let pcount = ctrl.get_parameter_count();
                for i in 0..pcount {
                    let mut pinfo: ParameterInfo = std::mem::zeroed();
                    if ctrl.get_parameter_info(i, &mut pinfo) == kResultOk {
                        params.push(Vst3ParamMeta {
                            id: pinfo.id,
                            title: utf16_from_i16(&pinfo.title),
                            short_title: utf16_from_i16(&pinfo.short_title),
                            default_normalized: pinfo.default_normalized_value,
                            step_count: pinfo.step_count,
                        });
                    }
                }
            }

            // Activate main stereo audio out bus & main event input bus
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

            let mut inst = Self {
                component: Some(component),
                processor: Some(processor),
                controller,
                _host_app: host_app,
                exit_dll,
                module,
                vendor,
                plugin_name,
                params,
                sample_rate,
                block_size,
                sample_pos: 0,
                bpm: 120.0,
            };
            inst.reconfigure(sample_rate, block_size)?;
            Ok(inst)
        }
    }

    pub fn reconfigure(&mut self, sample_rate: f64, block_size: usize) -> Result<(), String> {
        self.sample_rate = sample_rate;
        self.block_size = block_size;
        let processor = self.processor.as_ref().ok_or("processor dropped")?;
        let component = self.component.as_ref().ok_or("component dropped")?;
        unsafe {
            let _ = processor.set_processing(0);
            let _ = component.set_active(0);

            let setup = ProcessSetup {
                process_mode: ProcessModes::kRealtime as i32,
                symbolic_sample_size: SymbolicSampleSizes::kSample32 as i32,
                max_samples_per_block: block_size as i32,
                sample_rate,
            };
            let res = processor.setup_processing(&setup);
            if res != kResultOk {
                return Err(format!("setup_processing failed: {}", res));
            }
            let act_res = component.set_active(1);
            if act_res != kResultOk {
                return Err(format!("set_active(1) failed: {}", act_res));
            }
            let proc_res = processor.set_processing(1);
            if proc_res != kResultOk && proc_res != kResultFalse {
                return Err(format!("set_processing(1) failed: {}", proc_res));
            }
            Ok(())
        }
    }

    pub fn find_param_id(&self, needles: &[&str]) -> Option<u32> {
        for p in &self.params {
            let t = p.title.to_ascii_lowercase();
            let s = p.short_title.to_ascii_lowercase();
            if needles.iter().all(|n| {
                let nl = n.to_ascii_lowercase();
                t.contains(&nl) || s.contains(&nl)
            }) {
                return Some(p.id);
            }
        }
        None
    }

    pub fn get_state_bytes(&self) -> Result<Vec<u8>, String> {
        let component = self.component.as_ref().ok_or("component dropped")?;
        unsafe {
            let stream = HostMemStream::new(Vec::new());
            let raw_ptr: *const HostMemStream = &*stream;
            let shared: SharedVstPtr<dyn IBStream> = std::mem::transmute(raw_ptr);
            let res = component.get_state(shared);
            if res != kResultOk {
                return Err(format!("IComponent::get_state failed: {}", res));
            }
            Ok(stream.take_bytes())
        }
    }

    pub fn set_state_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        let component = self.component.as_ref().ok_or("component dropped")?;
        unsafe {
            let stream = HostMemStream::new(bytes.to_vec());
            let raw_ptr: *const HostMemStream = &*stream;
            let shared: SharedVstPtr<dyn IBStream> = std::mem::transmute(raw_ptr);
            let res = component.set_state(shared);
            if res != kResultOk {
                return Err(format!("IComponent::set_state failed: {}", res));
            }
            if let Some(ref ctrl) = self.controller {
                let stream2 = HostMemStream::new(bytes.to_vec());
                let raw2: *const HostMemStream = &*stream2;
                let shared2: SharedVstPtr<dyn IBStream> = std::mem::transmute(raw2);
                let _ = ctrl.set_component_state(shared2);
            }
            Ok(())
        }
    }

    pub fn process_block(
        &mut self,
        midi: &[MidiEvent],
        param_changes: &[(u32, f64)],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) -> Result<(), String> {
        let n = out_l.len().min(out_r.len());
        out_l[..n].fill(0.0);
        out_r[..n].fill(0.0);

        let mut vst_events = Vec::with_capacity(midi.len());
        for ev in midi {
            match *ev {
                MidiEvent::NoteOn {
                    sample_offset,
                    note,
                    velocity,
                } => {
                    vst_events.push(Event {
                        bus_index: 0,
                        sample_offset,
                        ppq_position: (self.sample_pos as f64 / self.sample_rate) * (self.bpm / 60.0),
                        flags: 0,
                        type_: EventTypes::kNoteOnEvent as u16,
                        event: EventData {
                            note_on: NoteOnEvent {
                                channel: 0,
                                pitch: note,
                                tuning: 0.0,
                                velocity,
                                length: 0,
                                note_id: -1,
                            },
                        },
                    });
                }
                MidiEvent::NoteOff {
                    sample_offset,
                    note,
                } => {
                    vst_events.push(Event {
                        bus_index: 0,
                        sample_offset,
                        ppq_position: (self.sample_pos as f64 / self.sample_rate) * (self.bpm / 60.0),
                        flags: 0,
                        type_: EventTypes::kNoteOffEvent as u16,
                        event: EventData {
                            note_off: NoteOffEvent {
                                channel: 0,
                                pitch: note,
                                velocity: 0.0,
                                note_id: -1,
                                tuning: 0.0,
                            },
                        },
                    });
                }
            }
        }

        unsafe {
            let ev_list = HostEventList::new(vst_events);
            let p_changes = HostParamChanges::new(param_changes.to_vec());
            let out_p_changes = HostParamChanges::new(Vec::new());
            let out_ev_list = HostEventList::new(Vec::new());

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
            ctx.state = 1 | (1 << 9) | (1 << 10); // kPlaying | kProjectTimeMusicValid | kTempoValid
            ctx.sample_rate = self.sample_rate;
            ctx.project_time_samples = self.sample_pos;
            ctx.project_time_music =
                (self.sample_pos as f64 / self.sample_rate) * (self.bpm / 60.0);
            ctx.tempo = self.bpm;
            ctx.time_sig_num = 4;
            ctx.time_sig_den = 4;

            let ev_ptr: *const HostEventList = &*ev_list;
            let pc_ptr: *const HostParamChanges = &*p_changes;
            let out_ev_ptr: *const HostEventList = &*out_ev_list;
            let out_pc_ptr: *const HostParamChanges = &*out_p_changes;

            let mut data = ProcessData {
                process_mode: ProcessModes::kRealtime as i32,
                symbolic_sample_size: SymbolicSampleSizes::kSample32 as i32,
                num_samples: n as i32,
                num_inputs: 0,
                num_outputs: 1,
                inputs: ptr::null_mut(),
                outputs: &mut out_bus,
                input_param_changes: std::mem::transmute(pc_ptr),
                output_param_changes: std::mem::transmute(out_pc_ptr),
                input_events: std::mem::transmute(ev_ptr),
                output_events: std::mem::transmute(out_ev_ptr),
                context: &mut ctx,
            };

            let processor = self.processor.as_ref().ok_or("processor dropped")?;
            let res = processor.process(&mut data);
            self.sample_pos += n as i64;
            if res != kResultOk {
                return Err(format!("IAudioProcessor::process failed: {}", res));
            }
            Ok(())
        }
    }

    pub fn render_notes(
        &mut self,
        notes: &[i16],
        hold_blocks: usize,
        release_blocks: usize,
        param_changes: &[(u32, f64)],
    ) -> Result<(Vec<f32>, Vec<f32>), String> {
        let bs = self.block_size;
        let total_blocks = hold_blocks + release_blocks;
        let mut all_l = Vec::with_capacity(total_blocks * bs);
        let mut all_r = Vec::with_capacity(total_blocks * bs);
        let mut blk_l = vec![0.0_f32; bs];
        let mut blk_r = vec![0.0_f32; bs];

        for b in 0..total_blocks {
            let mut evs = Vec::new();
            if b == 0 {
                for &n in notes {
                    evs.push(MidiEvent::NoteOn {
                        sample_offset: 0,
                        note: n,
                        velocity: 0.85,
                    });
                }
            }
            if b == hold_blocks {
                for &n in notes {
                    evs.push(MidiEvent::NoteOff {
                        sample_offset: 0,
                        note: n,
                    });
                }
            }
            let pcs = if b == 0 { param_changes } else { &[] };
            self.process_block(&evs, pcs, &mut blk_l, &mut blk_r)?;
            all_l.extend_from_slice(&blk_l);
            all_r.extend_from_slice(&blk_r);
        }
        Ok((all_l, all_r))
    }
}

impl Drop for Vst3PluginInstance {
    fn drop(&mut self) {
        unsafe {
            if let Some(ref p) = self.processor {
                let _ = p.set_processing(0);
            }
            if let Some(ref c) = self.component {
                let _ = c.set_active(0);
                let _ = c.terminate();
            }
            // Drop COM interfaces BEFORE ExitDll / FreeLibrary!
            self.controller.take();
            self.processor.take();
            self.component.take();
            if let Some(exit_fn) = self.exit_dll {
                let _ = exit_fn();
            }
            FreeLibrary(self.module);
        }
    }
}
