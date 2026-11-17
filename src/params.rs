use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const DEFAULT_EDITOR_WIDTH: u32 = 1220;
pub const DEFAULT_EDITOR_HEIGHT: u32 = 760;

// ============================================================================
// Enums (every variant has a stable #[id = "..."] matching its display name)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum PolyMode {
    #[id = "Poly"]
    Poly,
    #[id = "Mono 1"]
    #[name = "Mono 1"]
    Mono1,
    #[id = "Mono 2"]
    #[name = "Mono 2"]
    Mono2,
    #[id = "Mono 3"]
    #[name = "Mono 3"]
    Mono3,
    #[id = "Mono 4"]
    #[name = "Mono 4"]
    Mono4,
    #[id = "Legato"]
    Legato,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum MicrotuningMode {
    #[id = "Equal"]
    Equal,
    #[id = "Just"]
    Just,
    #[id = "Pythagorean"]
    Pythagorean,
    #[id = "Werckmeister 3"]
    #[name = "Werckmeister 3"]
    Werckmeister3,
    #[id = "Quarter Tone"]
    #[name = "Quarter Tone"]
    QuarterTone,
    #[id = "Maqam"]
    Maqam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum OscMode {
    #[id = "Classic"]
    Classic,
    #[id = "Noise"]
    Noise,
    #[id = "FM"]
    FM,
    #[id = "AMSync"]
    AMSync,
    #[id = "SawPWM"]
    SawPWM,
    #[id = "HardFM"]
    HardFM,
    #[id = "Vowel"]
    Vowel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum OscWaveform {
    #[id = "Sin"]
    Sin,
    #[id = "Tri"]
    Tri,
    #[id = "Saw"]
    Saw,
    #[id = "Square"]
    Square,
    #[id = "Pulse"]
    Pulse,
    #[id = "SuperSaw"]
    SuperSaw,
    #[id = "Organ"]
    Organ,
    #[id = "Bell"]
    Bell,
    #[id = "Formant"]
    Formant,
    #[id = "Metallic"]
    Metallic,
    #[id = "Acid"]
    Acid,
    #[id = "Bass"]
    Bass,
    #[id = "Choir"]
    Choir,
    #[id = "Strings"]
    Strings,
    #[id = "Hollow"]
    Hollow,
    #[id = "Sub"]
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum UnisonMode {
    #[id = "1 Octave"]
    #[name = "1 Octave"]
    Octave1,
    #[id = "2 Octaves"]
    #[name = "2 Octaves"]
    Octave2,
    #[id = "3 Octaves"]
    #[name = "3 Octaves"]
    Octave3,
    #[id = "Fifth"]
    Fifth,
    #[id = "Minor 3rd"]
    #[name = "Minor 3rd"]
    Minor3rd,
    #[id = "Major 3rd"]
    #[name = "Major 3rd"]
    Major3rd,
    #[id = "Sus4"]
    Sus4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum FilterModel {
    #[id = "Perfecto"]
    Perfecto,
    #[id = "Infecto"]
    Infecto,
    #[id = "Acido"]
    Acido,
    #[id = "Scorpio"]
    Scorpio,
    #[id = "Combo"]
    Combo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum FilterType {
    #[id = "LP2"]
    LP2,
    #[id = "LP4"]
    LP4,
    #[id = "LR4"]
    LR4,
    #[id = "BP2"]
    BP2,
    #[id = "BP4"]
    BP4,
    #[id = "HP2"]
    HP2,
    #[id = "HP4"]
    HP4,
    #[id = "Peak"]
    Peak,
    #[id = "Notch"]
    Notch,
    #[id = "LP+BP"]
    #[name = "LP+BP"]
    LpBp,
    #[id = "HP+BP"]
    #[name = "HP+BP"]
    HpBp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum SyncDiv {
    #[id = "1/64"]
    #[name = "1/64"]
    D1_64,
    #[id = "1/32"]
    #[name = "1/32"]
    D1_32,
    #[id = "1/16T"]
    #[name = "1/16T"]
    D1_16T,
    #[id = "1/16"]
    #[name = "1/16"]
    D1_16,
    #[id = "1/16D"]
    #[name = "1/16D"]
    D1_16D,
    #[id = "1/8T"]
    #[name = "1/8T"]
    D1_8T,
    #[id = "1/8"]
    #[name = "1/8"]
    D1_8,
    #[id = "1/8D"]
    #[name = "1/8D"]
    D1_8D,
    #[id = "1/4T"]
    #[name = "1/4T"]
    D1_4T,
    #[id = "1/4"]
    #[name = "1/4"]
    D1_4,
    #[id = "1/4D"]
    #[name = "1/4D"]
    D1_4D,
    #[id = "1/2T"]
    #[name = "1/2T"]
    D1_2T,
    #[id = "1/2"]
    #[name = "1/2"]
    D1_2,
    #[id = "1/2D"]
    #[name = "1/2D"]
    D1_2D,
    #[id = "1/1"]
    #[name = "1/1"]
    D1_1,
    #[id = "2/1"]
    #[name = "2/1"]
    D2_1,
    #[id = "4/1"]
    #[name = "4/1"]
    D4_1,
    #[id = "8/1"]
    #[name = "8/1"]
    D8_1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum LfoWaveform {
    #[id = "Sin"]
    Sin,
    #[id = "Tri"]
    Tri,
    #[id = "Saw Up"]
    #[name = "Saw Up"]
    SawUp,
    #[id = "Saw Down"]
    #[name = "Saw Down"]
    SawDn,
    #[id = "Square"]
    Square,
    #[id = "S&H"]
    #[name = "S&H"]
    Snh,
    #[id = "Smooth Rnd"]
    #[name = "Smooth Rnd"]
    SmoothRnd,
    #[id = "Ramp"]
    Ramp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ArpMode {
    #[id = "Up"]
    Up,
    #[id = "Down"]
    Down,
    #[id = "Up/Down"]
    #[name = "Up/Down"]
    UpDown,
    #[id = "Down/Up"]
    #[name = "Down/Up"]
    DownUp,
    #[id = "Random"]
    Random,
    #[id = "As Played"]
    #[name = "As Played"]
    AsPlayed,
    #[id = "Chord"]
    Chord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ArpVelMode {
    #[id = "Note Vel"]
    #[name = "Note Vel"]
    NoteVel,
    #[id = "Pattern Vel"]
    #[name = "Pattern Vel"]
    PatternVel,
    #[id = "Mixed"]
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ModSource {
    #[id = "Off"]
    Off,
    #[id = "ENV 1"]
    #[name = "ENV 1"]
    Env1,
    #[id = "ENV 2"]
    #[name = "ENV 2"]
    Env2,
    #[id = "ENV 3"]
    #[name = "ENV 3"]
    Env3,
    #[id = "ENV 4"]
    #[name = "ENV 4"]
    Env4,
    #[id = "LFO 1"]
    #[name = "LFO 1"]
    Lfo1,
    #[id = "LFO 2"]
    #[name = "LFO 2"]
    Lfo2,
    #[id = "LFO 3"]
    #[name = "LFO 3"]
    Lfo3,
    #[id = "LFO 4"]
    #[name = "LFO 4"]
    Lfo4,
    #[id = "STP 1"]
    #[name = "STP 1"]
    Stp1,
    #[id = "STP 2"]
    #[name = "STP 2"]
    Stp2,
    #[id = "Macro 1"]
    #[name = "Macro 1"]
    Macro1,
    #[id = "Macro 2"]
    #[name = "Macro 2"]
    Macro2,
    #[id = "Macro 3"]
    #[name = "Macro 3"]
    Macro3,
    #[id = "Macro 4"]
    #[name = "Macro 4"]
    Macro4,
    #[id = "Velocity"]
    Velocity,
    #[id = "Keytrack"]
    Keytrack,
    #[id = "Mod Wheel"]
    #[name = "Mod Wheel"]
    ModWheel,
    #[id = "Pitch Bend"]
    #[name = "Pitch Bend"]
    PitchBend,
    #[id = "Aftertouch"]
    Aftertouch,
    #[id = "Random"]
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ModTarget {
    #[id = "Off"]
    Off,
    #[id = "OSC1 Pitch"]
    #[name = "OSC1 Pitch"]
    Osc1Pitch,
    #[id = "OSC2 Pitch"]
    #[name = "OSC2 Pitch"]
    Osc2Pitch,
    #[id = "OSC3 Pitch"]
    #[name = "OSC3 Pitch"]
    Osc3Pitch,
    #[id = "OSC4 Pitch"]
    #[name = "OSC4 Pitch"]
    Osc4Pitch,
    #[id = "All Pitch"]
    #[name = "All Pitch"]
    AllOscPitch,
    #[id = "OSC1 CtrlA"]
    #[name = "OSC1 CtrlA"]
    Osc1CtrlA,
    #[id = "OSC2 CtrlA"]
    #[name = "OSC2 CtrlA"]
    Osc2CtrlA,
    #[id = "OSC3 CtrlA"]
    #[name = "OSC3 CtrlA"]
    Osc3CtrlA,
    #[id = "OSC4 CtrlA"]
    #[name = "OSC4 CtrlA"]
    Osc4CtrlA,
    #[id = "OSC1 CtrlB"]
    #[name = "OSC1 CtrlB"]
    Osc1CtrlB,
    #[id = "OSC2 CtrlB"]
    #[name = "OSC2 CtrlB"]
    Osc2CtrlB,
    #[id = "OSC3 CtrlB"]
    #[name = "OSC3 CtrlB"]
    Osc3CtrlB,
    #[id = "OSC4 CtrlB"]
    #[name = "OSC4 CtrlB"]
    Osc4CtrlB,
    #[id = "OSC1 WT Mix"]
    #[name = "OSC1 WT Mix"]
    Osc1WtMix,
    #[id = "OSC2 WT Mix"]
    #[name = "OSC2 WT Mix"]
    Osc2WtMix,
    #[id = "OSC3 WT Mix"]
    #[name = "OSC3 WT Mix"]
    Osc3WtMix,
    #[id = "OSC4 WT Mix"]
    #[name = "OSC4 WT Mix"]
    Osc4WtMix,
    #[id = "OSC1 Level"]
    #[name = "OSC1 Level"]
    Osc1Level,
    #[id = "OSC2 Level"]
    #[name = "OSC2 Level"]
    Osc2Level,
    #[id = "OSC3 Level"]
    #[name = "OSC3 Level"]
    Osc3Level,
    #[id = "OSC4 Level"]
    #[name = "OSC4 Level"]
    Osc4Level,
    #[id = "OSC1 Pan"]
    #[name = "OSC1 Pan"]
    Osc1Pan,
    #[id = "OSC2 Pan"]
    #[name = "OSC2 Pan"]
    Osc2Pan,
    #[id = "OSC3 Pan"]
    #[name = "OSC3 Pan"]
    Osc3Pan,
    #[id = "OSC4 Pan"]
    #[name = "OSC4 Pan"]
    Osc4Pan,
    #[id = "OSC1 Detune"]
    #[name = "OSC1 Detune"]
    Osc1Detune,
    #[id = "OSC2 Detune"]
    #[name = "OSC2 Detune"]
    Osc2Detune,
    #[id = "OSC3 Detune"]
    #[name = "OSC3 Detune"]
    Osc3Detune,
    #[id = "OSC4 Detune"]
    #[name = "OSC4 Detune"]
    Osc4Detune,
    #[id = "Filter1 Cutoff"]
    #[name = "Filter1 Cutoff"]
    Filter1Cutoff,
    #[id = "Filter1 Res"]
    #[name = "Filter1 Res"]
    Filter1Res,
    #[id = "Filter2 Cutoff"]
    #[name = "Filter2 Cutoff"]
    Filter2Cutoff,
    #[id = "Filter2 Res"]
    #[name = "Filter2 Res"]
    Filter2Res,
    #[id = "Filter Balance"]
    #[name = "Filter Balance"]
    FilterBalance,
    #[id = "ENV1 Attack"]
    #[name = "ENV1 Attack"]
    Env1Attack,
    #[id = "ENV1 Decay"]
    #[name = "ENV1 Decay"]
    Env1Decay,
    #[id = "ENV2 Attack"]
    #[name = "ENV2 Attack"]
    Env2Attack,
    #[id = "ENV2 Decay"]
    #[name = "ENV2 Decay"]
    Env2Decay,
    #[id = "LFO1 Rate"]
    #[name = "LFO1 Rate"]
    Lfo1Rate,
    #[id = "LFO1 Amp"]
    #[name = "LFO1 Amp"]
    Lfo1Amt,
    #[id = "LFO2 Rate"]
    #[name = "LFO2 Rate"]
    Lfo2Rate,
    #[id = "LFO2 Amp"]
    #[name = "LFO2 Amp"]
    Lfo2Amt,
    #[id = "FX Shaper Drive"]
    #[name = "FX Shaper Drive"]
    FxShaperDrive,
    #[id = "FX Shaper Mix"]
    #[name = "FX Shaper Mix"]
    FxShaperMix,
    #[id = "FX Phaser Freq"]
    #[name = "FX Phaser Freq"]
    FxPhaserFreq,
    #[id = "FX Phaser Mix"]
    #[name = "FX Phaser Mix"]
    FxPhaserMix,
    #[id = "FX Chorus Depth"]
    #[name = "FX Chorus Depth"]
    FxChorusDepth,
    #[id = "FX Chorus Mix"]
    #[name = "FX Chorus Mix"]
    FxChorusMix,
    #[id = "FX Delay FB"]
    #[name = "FX Delay FB"]
    FxDelayFeedback,
    #[id = "FX Delay Mix"]
    #[name = "FX Delay Mix"]
    FxDelayMix,
    #[id = "FX Reverb Decay"]
    #[name = "FX Reverb Decay"]
    FxReverbDecay,
    #[id = "FX Reverb Mix"]
    #[name = "FX Reverb Mix"]
    FxReverbMix,
    #[id = "Master Volume"]
    #[name = "Master Volume"]
    MasterVolume,
    #[id = "Global Pan"]
    #[name = "Global Pan"]
    Pan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ShaperMode {
    #[id = "Warm"]
    Warm,
    #[id = "Soft"]
    Soft,
    #[id = "Hard"]
    Hard,
    #[id = "Clip"]
    Clip,
    #[id = "Tube"]
    Tube,
    #[id = "Fold"]
    Fold,
    #[id = "Sine"]
    Sine,
    #[id = "Bitcrush"]
    Bitcrush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ChorusMode {
    #[id = "Chorus"]
    Chorus,
    #[id = "Ensemble"]
    Ensemble,
    #[id = "Dimension"]
    Dimension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ReverbMode {
    #[id = "Plate 1"]
    #[name = "Plate 1"]
    Plate1,
    #[id = "Plate 2"]
    #[name = "Plate 2"]
    Plate2,
    #[id = "Hall"]
    Hall,
    #[id = "Room"]
    Room,
    #[id = "Ambient"]
    Ambient,
}

// ============================================================================
// Non-Param Persistent Synth State (mirrored lock-free in SharedSynthState)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtraSynthState {
    pub preset_name: String,
    pub preset_category: String,
    pub preset_author: String,
    pub stepper_steps: [[f32; 16]; 2],
    pub stepper_ties: [u16; 2],
    pub arp_steps_enabled: u16,
    pub arp_step_offsets: [i8; 16],
    pub arp_step_velocities: [u8; 16],
    pub mod_sources: [u8; 15],
    pub mod_targets: [u8; 15],
    pub microtuning_preset: u8,
    pub microtuning_cents: [f32; 12],
    #[serde(skip)]
    pub osc_copy_buffer: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(skip)]
    pub env_copy_buffer: Option<BTreeMap<String, serde_json::Value>>,
}

impl Default for ExtraSynthState {
    fn default() -> Self {
        Self {
            preset_name: String::from("Init"),
            preset_category: String::from("Init"),
            preset_author: String::from("Phantom Spire"),
            stepper_steps: [[0.5; 16]; 2],
            stepper_ties: [0; 2],
            arp_steps_enabled: 0xFFFF,
            arp_step_offsets: [0; 16],
            arp_step_velocities: [100; 16],
            mod_sources: [0; 15],
            mod_targets: [0; 15],
            microtuning_preset: 0,
            microtuning_cents: [0.0; 12],
            osc_copy_buffer: None,
            env_copy_buffer: None,
        }
    }
}

// ============================================================================
// Nested Parameter Structs
// ============================================================================

#[derive(Params)]
pub struct MacroParam {
    #[id = "g_mac"]
    pub value: FloatParam,
}

impl MacroParam {
    pub fn new(idx: usize) -> Self {
        Self {
            value: FloatParam::new(
                format!("Macro {}", idx + 1),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct GlobalParams {
    #[id = "g_voc"]
    pub voices: IntParam,

    #[id = "g_pmod"]
    pub poly_mode: EnumParam<PolyMode>,

    #[id = "g_trns"]
    pub transpose: IntParam,

    #[id = "g_gld"]
    pub glide: FloatParam,

    #[id = "g_glog"]
    pub glide_log: BoolParam,

    #[id = "g_bup"]
    pub bender_up: IntParam,

    #[id = "g_bdn"]
    pub bender_down: IntParam,

    #[id = "g_drft"]
    pub drift: BoolParam,

    #[id = "g_pw"]
    pub pitch_wheel: FloatParam,

    #[id = "g_mw"]
    pub mod_wheel: FloatParam,

    #[id = "g_utun"]
    pub microtuning: EnumParam<MicrotuningMode>,

    #[id = "host_bpm_sync"]
    pub host_bpm_sync: BoolParam,

    #[nested(array)]
    pub macros: [MacroParam; 4],
}

impl Default for GlobalParams {
    fn default() -> Self {
        Self {
            voices: IntParam::new("Voices", 16, IntRange::Linear { min: 1, max: 16 }),
            poly_mode: EnumParam::new("Poly Mode", PolyMode::Poly),
            transpose: IntParam::new("Transpose", 0, IntRange::Linear { min: -24, max: 24 })
                .with_unit(" st"),
            glide: FloatParam::new(
                "Glide",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            glide_log: BoolParam::new("Glide Log", false),
            bender_up: IntParam::new("Bender Up", 2, IntRange::Linear { min: 0, max: 24 })
                .with_unit(" st"),
            bender_down: IntParam::new("Bender Down", 2, IntRange::Linear { min: 0, max: 24 })
                .with_unit(" st"),
            drift: BoolParam::new("Drift", false),
            pitch_wheel: FloatParam::new(
                "Pitch Wheel",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            mod_wheel: FloatParam::new(
                "Mod Wheel",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            microtuning: EnumParam::new("Microtuning", MicrotuningMode::Equal),
            host_bpm_sync: BoolParam::new("Project BPM Sync", true),
            macros: [
                MacroParam::new(0),
                MacroParam::new(1),
                MacroParam::new(2),
                MacroParam::new(3),
            ],
        }
    }
}

#[derive(Params)]
pub struct OscParams {
    #[id = "o_mod"]
    pub mode: EnumParam<OscMode>,
    #[id = "o_wav"]
    pub waveform: EnumParam<OscWaveform>,
    #[id = "o_wtm"]
    pub wt_mix: FloatParam,
    #[id = "o_not"]
    pub note: IntParam,
    #[id = "o_fin"]
    pub fine: FloatParam,
    #[id = "o_oct"]
    pub octave: IntParam,
    #[id = "o_ca"]
    pub ctrl_a: FloatParam,
    #[id = "o_cb"]
    pub ctrl_b: FloatParam,
    #[id = "o_phs"]
    pub phase: FloatParam,
    #[id = "o_ucnt"]
    pub unison_count: IntParam,
    #[id = "o_udet"]
    pub unison_detune: FloatParam,
    #[id = "o_udns"]
    pub unison_density: FloatParam,
    #[id = "o_umod"]
    pub unison_mode: EnumParam<UnisonMode>,
    #[id = "o_wid"]
    pub wide: FloatParam,
    #[id = "o_pan"]
    pub pan: FloatParam,
    #[id = "o_lvl"]
    pub mix_level: FloatParam,
    #[id = "o_fbal"]
    pub filter_balance: FloatParam,
    #[id = "o_ana"]
    pub ana: BoolParam,
    #[id = "o_inv"]
    pub inv: BoolParam,
    #[id = "o_key"]
    pub key: BoolParam,
}

impl OscParams {
    pub fn new(idx: usize) -> Self {
        let p = idx + 1;
        let default_lvl = if idx == 0 { 0.8 } else { 0.0 };
        Self {
            mode: EnumParam::new(format!("OSC{p} Mode"), OscMode::Classic),
            waveform: EnumParam::new(format!("OSC{p} Wave"), OscWaveform::Saw),
            wt_mix: FloatParam::new(
                format!("OSC{p} WT Mix"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            note: IntParam::new(
                format!("OSC{p} Note"),
                0,
                IntRange::Linear { min: -24, max: 24 },
            )
            .with_unit(" st"),
            fine: FloatParam::new(
                format!("OSC{p} Fine"),
                0.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            octave: IntParam::new(
                format!("OSC{p} Octave"),
                0,
                IntRange::Linear { min: -4, max: 4 },
            ),
            ctrl_a: FloatParam::new(
                format!("OSC{p} Ctrl A"),
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            ctrl_b: FloatParam::new(
                format!("OSC{p} Ctrl B"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            phase: FloatParam::new(
                format!("OSC{p} Phase"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            unison_count: IntParam::new(
                format!("OSC{p} Unison Voices"),
                1,
                IntRange::Linear { min: 1, max: 9 },
            ),
            unison_detune: FloatParam::new(
                format!("OSC{p} Detune"),
                0.2,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            unison_density: FloatParam::new(
                format!("OSC{p} Density"),
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            unison_mode: EnumParam::new(format!("OSC{p} Unison Mode"), UnisonMode::Octave1),
            wide: FloatParam::new(
                format!("OSC{p} Wide"),
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            pan: FloatParam::new(
                format!("OSC{p} Pan"),
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_panning())
            .with_string_to_value(formatters::s2v_f32_panning()),
            mix_level: FloatParam::new(
                format!("OSC{p} Mix"),
                default_lvl,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            filter_balance: FloatParam::new(
                format!("OSC{p} Filter Input"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            ana: BoolParam::new(format!("OSC{p} ANA"), false),
            inv: BoolParam::new(format!("OSC{p} INV"), false),
            key: BoolParam::new(format!("OSC{p} KEY"), true),
        }
    }
}

#[derive(Params)]
pub struct FilterParams {
    #[id = "f_mod"]
    pub model: EnumParam<FilterModel>,
    #[id = "f_typ"]
    pub filter_type: EnumParam<FilterType>,
    #[id = "f_cut"]
    pub cutoff: FloatParam,
    #[id = "f_res"]
    pub resonance: FloatParam,
    #[id = "f_ktr"]
    pub keytrack: FloatParam,
    #[id = "f_drv"]
    pub drive: FloatParam,
}

impl FilterParams {
    pub fn new(idx: usize) -> Self {
        let p = idx + 1;
        let (default_type, default_cut) = if idx == 0 {
            (FilterType::LP4, 18000.0)
        } else {
            (FilterType::HP4, 20.0)
        };
        Self {
            model: EnumParam::new(format!("Filter {p} Model"), FilterModel::Perfecto),
            filter_type: EnumParam::new(format!("Filter {p} Type"), default_type),
            cutoff: FloatParam::new(
                format!("Filter {p} Cutoff"),
                default_cut,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.2),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            resonance: FloatParam::new(
                format!("Filter {p} Res"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            keytrack: FloatParam::new(
                format!("Filter {p} Keytrack"),
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            drive: FloatParam::new(
                format!("Filter {p} Drive"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct FilterRoutingParams {
    #[id = "fr_bal"]
    pub filter_balance: FloatParam,
    #[id = "fr_par"]
    pub parallel: BoolParam,
    #[id = "fr_lnk"]
    pub link: BoolParam,
}

impl Default for FilterRoutingParams {
    fn default() -> Self {
        Self {
            filter_balance: FloatParam::new(
                "Filter Balance",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            parallel: BoolParam::new("Filter Parallel", false),
            link: BoolParam::new("Filter Link", false),
        }
    }
}

#[derive(Params)]
pub struct EnvModSlot {
    #[id = "es_tgt"]
    pub target: EnumParam<ModTarget>,
    #[id = "es_amt"]
    pub amt: FloatParam,
    #[id = "es_vel"]
    pub vel: FloatParam,
}

impl EnvModSlot {
    pub fn new(env_idx: usize, slot_idx: usize) -> Self {
        let e = env_idx + 1;
        let s = slot_idx + 1;
        Self {
            target: EnumParam::new(format!("ENV{e} Slot{s} Target"), ModTarget::Off),
            amt: FloatParam::new(
                format!("ENV{e} Slot{s} Amt"),
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            vel: FloatParam::new(
                format!("ENV{e} Slot{s} Vel"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct EnvParams {
    #[id = "e_att"]
    pub attack: FloatParam,
    #[id = "e_dec"]
    pub decay: FloatParam,
    #[id = "e_sus"]
    pub sustain: FloatParam,
    #[id = "e_slt"]
    pub slt: FloatParam,
    #[id = "e_sll"]
    pub sll: FloatParam,
    #[id = "e_rel"]
    pub release: FloatParam,
    #[id = "e_vel"]
    pub vel: FloatParam,

    #[nested(array)]
    pub mod_slots: [EnvModSlot; 2],
}

impl EnvParams {
    pub fn new(idx: usize) -> Self {
        let e = idx + 1;
        let (default_sus, default_vel) = if idx == 0 { (0.8, 0.5) } else { (0.0, 0.0) };
        Self {
            attack: FloatParam::new(
                format!("ENV{e} Attack"),
                0.002,
                FloatRange::Skewed {
                    min: 0.0005,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            decay: FloatParam::new(
                format!("ENV{e} Decay"),
                0.4,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 15.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            sustain: FloatParam::new(
                format!("ENV{e} Sustain"),
                default_sus,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            slt: FloatParam::new(
                format!("ENV{e} Slope Time"),
                0.5,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 15.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            sll: FloatParam::new(
                format!("ENV{e} Slope Level"),
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            release: FloatParam::new(
                format!("ENV{e} Release"),
                0.15,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 15.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            vel: FloatParam::new(
                format!("ENV{e} Velocity"),
                default_vel,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            mod_slots: [EnvModSlot::new(idx, 0), EnvModSlot::new(idx, 1)],
        }
    }
}

#[derive(Params)]
pub struct LfoModSlot {
    #[id = "ls_tgt"]
    pub target: EnumParam<ModTarget>,
    #[id = "ls_amt"]
    pub amt: FloatParam,
}

impl LfoModSlot {
    pub fn new(lfo_idx: usize, slot_idx: usize) -> Self {
        let l = lfo_idx + 1;
        let s = slot_idx + 1;
        Self {
            target: EnumParam::new(format!("LFO{l} Slot{s} Target"), ModTarget::Off),
            amt: FloatParam::new(
                format!("LFO{l} Slot{s} Amt"),
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct LfoParams {
    #[id = "l_rat"]
    pub rate: FloatParam,
    #[id = "l_syn"]
    pub sync: BoolParam,
    #[id = "l_sdiv"]
    pub sync_div: EnumParam<SyncDiv>,
    #[id = "l_wav"]
    pub waveform: EnumParam<LfoWaveform>,
    #[id = "l_mrp"]
    pub morph: FloatParam,
    #[id = "l_sym"]
    pub sym: FloatParam,
    #[id = "l_phs"]
    pub phase: FloatParam,
    #[id = "l_fad"]
    pub fade: FloatParam,
    #[id = "l_uni"]
    pub unipolar: BoolParam,
    #[id = "l_mon"]
    pub mono: BoolParam,

    #[nested(array)]
    pub mod_slots: [LfoModSlot; 2],
}

impl LfoParams {
    pub fn new(idx: usize) -> Self {
        let l = idx + 1;
        Self {
            rate: FloatParam::new(
                format!("LFO{l} Rate"),
                2.0,
                FloatRange::Skewed {
                    min: 0.01,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            sync: BoolParam::new(format!("LFO{l} Sync"), false),
            sync_div: EnumParam::new(format!("LFO{l} Sync Div"), SyncDiv::D1_4),
            waveform: EnumParam::new(format!("LFO{l} Wave"), LfoWaveform::Sin),
            morph: FloatParam::new(
                format!("LFO{l} Morph"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            sym: FloatParam::new(
                format!("LFO{l} Sym"),
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            phase: FloatParam::new(
                format!("LFO{l} Phase"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            fade: FloatParam::new(
                format!("LFO{l} Fade"),
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            unipolar: BoolParam::new(format!("LFO{l} Unipolar"), false),
            mono: BoolParam::new(format!("LFO{l} Mono"), false),
            mod_slots: [LfoModSlot::new(idx, 0), LfoModSlot::new(idx, 1)],
        }
    }
}

#[derive(Params)]
pub struct StepperParams {
    #[id = "s_rat"]
    pub rate: FloatParam,
    #[id = "s_syn"]
    pub sync: BoolParam,
    #[id = "s_sdiv"]
    pub sync_div: EnumParam<SyncDiv>,
    #[id = "s_gat"]
    pub gate: FloatParam,
    #[id = "s_smt"]
    pub smooth: FloatParam,
    #[id = "s_cnt"]
    pub steps_count: IntParam,
    #[id = "s_lop"]
    pub loop_mode: BoolParam,
}

impl StepperParams {
    pub fn new(idx: usize) -> Self {
        let s = idx + 1;
        Self {
            rate: FloatParam::new(
                format!("STP{s} Rate"),
                4.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 30.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            sync: BoolParam::new(format!("STP{s} Sync"), true),
            sync_div: EnumParam::new(format!("STP{s} Sync Div"), SyncDiv::D1_16),
            gate: FloatParam::new(
                format!("STP{s} Gate"),
                0.8,
                FloatRange::Linear {
                    min: 0.05,
                    max: 1.0,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            smooth: FloatParam::new(
                format!("STP{s} Smooth"),
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            steps_count: IntParam::new(
                format!("STP{s} Steps"),
                16,
                IntRange::Linear { min: 2, max: 16 },
            ),
            loop_mode: BoolParam::new(format!("STP{s} Loop"), true),
        }
    }
}

#[derive(Params)]
pub struct ArpParams {
    #[id = "a_on"]
    pub enabled: BoolParam,
    #[id = "a_mod"]
    pub mode: EnumParam<ArpMode>,
    #[id = "a_oct"]
    pub octaves: IntParam,
    #[id = "a_rat"]
    pub rate: FloatParam,
    #[id = "a_syn"]
    pub sync: BoolParam,
    #[id = "a_sdiv"]
    pub sync_div: EnumParam<SyncDiv>,
    #[id = "a_gat"]
    pub gate: FloatParam,
    #[id = "a_swg"]
    pub swing: FloatParam,
    #[id = "a_lat"]
    pub latch: BoolParam,
    #[id = "a_vel"]
    pub vel_mode: EnumParam<ArpVelMode>,
    #[id = "a_cnt"]
    pub steps_count: IntParam,
    #[id = "a_bpm"]
    pub internal_bpm: FloatParam,
}

impl Default for ArpParams {
    fn default() -> Self {
        Self {
            enabled: BoolParam::new("Arp On", false),
            mode: EnumParam::new("Arp Mode", ArpMode::Up),
            octaves: IntParam::new("Arp Octaves", 1, IntRange::Linear { min: 1, max: 4 }),
            rate: FloatParam::new(
                "Arp Rate",
                4.0,
                FloatRange::Skewed {
                    min: 0.5,
                    max: 30.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            sync: BoolParam::new("Arp Sync", true),
            sync_div: EnumParam::new("Arp Sync Div", SyncDiv::D1_16),
            gate: FloatParam::new(
                "Arp Gate",
                0.75,
                FloatRange::Linear {
                    min: 0.05,
                    max: 1.0,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            swing: FloatParam::new(
                "Arp Swing",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.75,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            latch: BoolParam::new("Arp Latch", false),
            vel_mode: EnumParam::new("Arp Vel Mode", ArpVelMode::NoteVel),
            steps_count: IntParam::new("Arp Steps", 16, IntRange::Linear { min: 1, max: 16 }),
            internal_bpm: FloatParam::new(
                "Internal BPM",
                120.0,
                FloatRange::Linear {
                    min: 40.0,
                    max: 240.0,
                },
            )
            .with_unit(" BPM")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

#[derive(Params)]
pub struct ModSlotParams {
    #[id = "m_src"]
    pub source: EnumParam<ModSource>,
    #[id = "m_tgt"]
    pub target: EnumParam<ModTarget>,
    #[id = "m_amt"]
    pub amount: FloatParam,
}

impl ModSlotParams {
    pub fn new(idx: usize) -> Self {
        let s = idx + 1;
        Self {
            source: EnumParam::new(format!("Mod {s} Source"), ModSource::Off),
            target: EnumParam::new(format!("Mod {s} Target"), ModTarget::Off),
            amount: FloatParam::new(
                format!("Mod {s} Amount"),
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct ShaperParams {
    #[id = "x_smut"]
    pub mute: BoolParam,
    #[id = "x_smod"]
    pub mode: EnumParam<ShaperMode>,
    #[id = "x_sbnd"]
    pub band: BoolParam,
    #[id = "x_shq"]
    pub hq: BoolParam,
    #[id = "x_sdrv"]
    pub drive: FloatParam,
    #[id = "x_sbit"]
    pub bit: FloatParam,
    #[id = "x_ssrt"]
    pub s_rate: FloatParam,
    #[id = "x_sloc"]
    pub low_cut: FloatParam,
    #[id = "x_shic"]
    pub hi_cut: FloatParam,
    #[id = "x_sdw"]
    pub dry_wet: FloatParam,
}

impl Default for ShaperParams {
    fn default() -> Self {
        Self {
            mute: BoolParam::new("Shaper Mute", false),
            mode: EnumParam::new("Shaper Mode", ShaperMode::Warm),
            band: BoolParam::new("Shaper Band", false),
            hq: BoolParam::new("Shaper HQ", false),
            drive: FloatParam::new(
                "Shaper Drive",
                0.35,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            bit: FloatParam::new(
                "Shaper Bit",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            s_rate: FloatParam::new(
                "Shaper S.Rate",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            low_cut: FloatParam::new(
                "Shaper Low Cut",
                20.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            hi_cut: FloatParam::new(
                "Shaper Hi Cut",
                20000.0,
                FloatRange::Skewed {
                    min: 1000.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            dry_wet: FloatParam::new(
                "Shaper Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct PhaserParams {
    #[id = "x_pmut"]
    pub mute: BoolParam,
    #[id = "x_pstg"]
    pub stages: IntParam,
    #[id = "x_prat"]
    pub rate: FloatParam,
    #[id = "x_pfb"]
    pub feedback: FloatParam,
    #[id = "x_pfrq"]
    pub freq: FloatParam,
    #[id = "x_pspr"]
    pub spread: FloatParam,
    #[id = "x_pdw"]
    pub dry_wet: FloatParam,
}

impl Default for PhaserParams {
    fn default() -> Self {
        Self {
            mute: BoolParam::new("Phaser Mute", false),
            stages: IntParam::new("Phaser Stages", 6, IntRange::Linear { min: 2, max: 12 }),
            rate: FloatParam::new(
                "Phaser Rate",
                0.5,
                FloatRange::Skewed {
                    min: 0.05,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            feedback: FloatParam::new(
                "Phaser Feedback",
                0.4,
                FloatRange::Linear {
                    min: -0.95,
                    max: 0.95,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            freq: FloatParam::new(
                "Phaser Freq",
                800.0,
                FloatRange::Skewed {
                    min: 100.0,
                    max: 8000.0,
                    factor: FloatRange::skew_factor(-1.8),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            spread: FloatParam::new(
                "Phaser Spread",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            dry_wet: FloatParam::new(
                "Phaser Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct ChorusParams {
    #[id = "x_cmut"]
    pub mute: BoolParam,
    #[id = "x_cmod"]
    pub mode: EnumParam<ChorusMode>,
    #[id = "x_crat"]
    pub rate: FloatParam,
    #[id = "x_cdel"]
    pub delay: FloatParam,
    #[id = "x_cdpt"]
    pub depth: FloatParam,
    #[id = "x_cfb"]
    pub feedback: FloatParam,
    #[id = "x_cloc"]
    pub low_cut: FloatParam,
    #[id = "x_chic"]
    pub hi_cut: FloatParam,
    #[id = "x_cdw"]
    pub dry_wet: FloatParam,
}

impl Default for ChorusParams {
    fn default() -> Self {
        Self {
            mute: BoolParam::new("Chorus Mute", false),
            mode: EnumParam::new("Chorus Mode", ChorusMode::Chorus),
            rate: FloatParam::new(
                "Chorus Rate",
                0.6,
                FloatRange::Skewed {
                    min: 0.05,
                    max: 8.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            delay: FloatParam::new(
                "Chorus Delay",
                10.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 40.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            depth: FloatParam::new(
                "Chorus Depth",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            feedback: FloatParam::new(
                "Chorus Feedback",
                0.15,
                FloatRange::Linear {
                    min: -0.9,
                    max: 0.9,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            low_cut: FloatParam::new(
                "Chorus Low Cut",
                20.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            hi_cut: FloatParam::new(
                "Chorus Hi Cut",
                16000.0,
                FloatRange::Skewed {
                    min: 1000.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            dry_wet: FloatParam::new(
                "Chorus Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct DelayParams {
    #[id = "x_dmut"]
    pub mute: BoolParam,
    #[id = "x_dsyn"]
    pub sync: BoolParam,
    #[id = "x_dtl"]
    pub time_l: FloatParam,
    #[id = "x_dtr"]
    pub time_r: FloatParam,
    #[id = "x_dsdl"]
    pub sync_div_l: EnumParam<SyncDiv>,
    #[id = "x_dsdr"]
    pub sync_div_r: EnumParam<SyncDiv>,
    #[id = "x_dfb"]
    pub feedback: FloatParam,
    #[id = "x_dpp"]
    pub ping_pong: BoolParam,
    #[id = "x_drat"]
    pub rate: FloatParam,
    #[id = "x_ddpt"]
    pub depth: FloatParam,
    #[id = "x_dloc"]
    pub low_cut: FloatParam,
    #[id = "x_dhic"]
    pub hi_cut: FloatParam,
    #[id = "x_ddw"]
    pub dry_wet: FloatParam,
}

impl Default for DelayParams {
    fn default() -> Self {
        Self {
            mute: BoolParam::new("Delay Mute", false),
            sync: BoolParam::new("Delay Sync", true),
            time_l: FloatParam::new(
                "Delay Time L",
                375.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            time_r: FloatParam::new(
                "Delay Time R",
                500.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            sync_div_l: EnumParam::new("Delay Sync L", SyncDiv::D1_8D),
            sync_div_r: EnumParam::new("Delay Sync R", SyncDiv::D1_4),
            feedback: FloatParam::new(
                "Delay Feedback",
                0.4,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            ping_pong: BoolParam::new("Delay Ping Pong", true),
            rate: FloatParam::new(
                "Delay Mod Rate",
                0.4,
                FloatRange::Skewed {
                    min: 0.05,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: FloatParam::new(
                "Delay Mod Depth",
                0.1,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            low_cut: FloatParam::new(
                "Delay Low Cut",
                80.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            hi_cut: FloatParam::new(
                "Delay Hi Cut",
                8000.0,
                FloatRange::Skewed {
                    min: 1000.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            dry_wet: FloatParam::new(
                "Delay Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params)]
pub struct ReverbParams {
    #[id = "x_rmut"]
    pub mute: BoolParam,
    #[id = "x_rmod"]
    pub mode: EnumParam<ReverbMode>,
    #[id = "x_rpre"]
    pub predelay: FloatParam,
    #[id = "x_rdec"]
    pub decay: FloatParam,
    #[id = "x_rdmp"]
    pub damp: FloatParam,
    #[id = "x_rcol"]
    pub color: FloatParam,
    #[id = "x_rwid"]
    pub wide: FloatParam,
    #[id = "x_rdw"]
    pub dry_wet: FloatParam,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            mute: BoolParam::new("Reverb Mute", false),
            mode: EnumParam::new("Reverb Mode", ReverbMode::Plate1),
            predelay: FloatParam::new(
                "Reverb PreDelay",
                15.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 250.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            decay: FloatParam::new(
                "Reverb Decay",
                2.5,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 12.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            damp: FloatParam::new(
                "Reverb Damp",
                0.4,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            color: FloatParam::new(
                "Reverb Color",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            wide: FloatParam::new(
                "Reverb Wide",
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            dry_wet: FloatParam::new(
                "Reverb Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

#[derive(Params, Default)]
pub struct FxParams {
    #[nested(group = "Shaper")]
    pub shaper: ShaperParams,
    #[nested(group = "Phaser")]
    pub phaser: PhaserParams,
    #[nested(group = "Chorus")]
    pub chorus: ChorusParams,
    #[nested(group = "Delay")]
    pub delay: DelayParams,
    #[nested(group = "Reverb")]
    pub reverb: ReverbParams,
}

#[derive(Params)]
pub struct MasterParams {
    #[id = "k_xcmp"]
    pub x_comp: FloatParam,
    #[id = "k_warm"]
    pub warm: BoolParam,
    #[id = "k_soft"]
    pub soft: BoolParam,
    #[id = "k_boos"]
    pub boost: BoolParam,
    #[id = "k_eqon"]
    pub eq_on: BoolParam,
    #[id = "k_elf"]
    pub eq_low_freq: FloatParam,
    #[id = "k_elq"]
    pub eq_low_q: FloatParam,
    #[id = "k_elg"]
    pub eq_low_gain: FloatParam,
    #[id = "k_emf"]
    pub eq_mid_freq: FloatParam,
    #[id = "k_emq"]
    pub eq_mid_q: FloatParam,
    #[id = "k_emg"]
    pub eq_mid_gain: FloatParam,
    #[id = "k_ehf"]
    pub eq_high_freq: FloatParam,
    #[id = "k_ehq"]
    pub eq_high_q: FloatParam,
    #[id = "k_ehg"]
    pub eq_high_gain: FloatParam,
    #[id = "k_vel"]
    pub velocity: FloatParam,
    #[id = "k_vol"]
    pub volume: FloatParam,
    #[id = "k_lim"]
    pub soft_limit: BoolParam,
}

impl Default for MasterParams {
    fn default() -> Self {
        Self {
            x_comp: FloatParam::new(
                "X-Comp",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            warm: BoolParam::new("Warm", false),
            soft: BoolParam::new("Soft", false),
            boost: BoolParam::new("Boost", false),
            eq_on: BoolParam::new("EQ On", false),
            eq_low_freq: FloatParam::new(
                "EQ Low Freq",
                100.0,
                FloatRange::Skewed {
                    min: 30.0,
                    max: 600.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            eq_low_q: FloatParam::new(
                "EQ Low Q",
                0.7,
                FloatRange::Skewed {
                    min: 0.3,
                    max: 4.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            eq_low_gain: FloatParam::new(
                "EQ Low Gain",
                0.0,
                FloatRange::Linear {
                    min: -15.0,
                    max: 15.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            eq_mid_freq: FloatParam::new(
                "EQ Mid Freq",
                1000.0,
                FloatRange::Skewed {
                    min: 150.0,
                    max: 8000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            eq_mid_q: FloatParam::new(
                "EQ Mid Q",
                1.0,
                FloatRange::Skewed {
                    min: 0.3,
                    max: 8.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            eq_mid_gain: FloatParam::new(
                "EQ Mid Gain",
                0.0,
                FloatRange::Linear {
                    min: -15.0,
                    max: 15.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            eq_high_freq: FloatParam::new(
                "EQ High Freq",
                8000.0,
                FloatRange::Skewed {
                    min: 1500.0,
                    max: 18000.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            eq_high_q: FloatParam::new(
                "EQ High Q",
                0.7,
                FloatRange::Skewed {
                    min: 0.3,
                    max: 4.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            eq_high_gain: FloatParam::new(
                "EQ High Gain",
                0.0,
                FloatRange::Linear {
                    min: -15.0,
                    max: 15.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            velocity: FloatParam::new(
                "Master Velocity",
                0.75,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            volume: {
                let default_vol = util::db_to_gain(-3.0);
                let param = FloatParam::new(
                    "Master Volume",
                    default_vol,
                    FloatRange::Skewed {
                        min: util::db_to_gain(-60.0),
                        max: util::db_to_gain(6.0),
                        factor: FloatRange::gain_skew_factor(-60.0, 6.0),
                    },
                )
                .with_smoother(SmoothingStyle::Logarithmic(10.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
                .with_string_to_value(formatters::s2v_f32_gain_to_db());
                param.smoothed.reset(default_vol);
                param
            },
            soft_limit: BoolParam::new("Soft Limit", true),
        }
    }
}

// ============================================================================
// Top-Level PhantomSpireParams
// ============================================================================

#[derive(Params)]
pub struct PhantomSpireParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[persist = "synth-non-param-state"]
    pub extra_state: Arc<Mutex<ExtraSynthState>>,

    #[nested(group = "Global")]
    pub global: GlobalParams,

    #[nested(array, group = "Osc")]
    pub oscs: [OscParams; 4],

    #[nested(array, group = "Filter")]
    pub filters: [FilterParams; 2],

    #[nested(group = "Filter Routing")]
    pub filter_routing: FilterRoutingParams,

    #[nested(array, group = "Env")]
    pub envs: [EnvParams; 4],

    #[nested(array, group = "LFO")]
    pub lfos: [LfoParams; 4],

    #[nested(array, group = "Stepper")]
    pub steppers: [StepperParams; 2],

    #[nested(group = "Arp")]
    pub arp: ArpParams,

    #[nested(array, group = "Mod Matrix")]
    pub mod_slots: [ModSlotParams; 15],

    #[nested(group = "FX")]
    pub fx: FxParams,

    #[nested(group = "Master")]
    pub master: MasterParams,
}

impl Default for PhantomSpireParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(DEFAULT_EDITOR_WIDTH, DEFAULT_EDITOR_HEIGHT),
            extra_state: Arc::new(Mutex::new(ExtraSynthState::default())),
            global: GlobalParams::default(),
            oscs: [
                OscParams::new(0),
                OscParams::new(1),
                OscParams::new(2),
                OscParams::new(3),
            ],
            filters: [FilterParams::new(0), FilterParams::new(1)],
            filter_routing: FilterRoutingParams::default(),
            envs: [
                EnvParams::new(0),
                EnvParams::new(1),
                EnvParams::new(2),
                EnvParams::new(3),
            ],
            lfos: [
                LfoParams::new(0),
                LfoParams::new(1),
                LfoParams::new(2),
                LfoParams::new(3),
            ],
            steppers: [StepperParams::new(0), StepperParams::new(1)],
            arp: ArpParams::default(),
            mod_slots: std::array::from_fn(ModSlotParams::new),
            fx: FxParams::default(),
            master: MasterParams::default(),
        }
    }
}

/// Mirror `nih_plug`'s internal 31-bit parameter ID hash function (`src/wrapper/util.rs`).
pub fn hash_param_id(id: &str) -> u32 {
    let mut hash: u32 = 0;
    for byte in id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }
    hash &= !(1 << 31);
    hash
}

/// Safe helper for headless/integration tests to set a `FloatParam` plain value (and reset its smoother).
pub fn set_float_param(param: &mut FloatParam, value: f32) {
    let name = param.name().to_string();
    let min = value.min(-1_000_000.0);
    let max = value.max(1_000_000.0);
    let p = FloatParam::new(name, value, FloatRange::Linear { min, max });
    p.smoothed.reset(value);
    *param = p;
}

/// Safe helper for headless/integration tests to set an `IntParam` plain value.
pub fn set_int_param(param: &mut IntParam, value: i32) {
    let name = param.name().to_string();
    let min = value.min(-1_000_000);
    let max = value.max(1_000_000);
    *param = IntParam::new(name, value, IntRange::Linear { min, max });
}

/// Safe helper for headless/integration tests to set a `BoolParam` plain value.
pub fn set_bool_param(param: &mut BoolParam, value: bool) {
    let name = param.name().to_string();
    *param = BoolParam::new(name, value);
}

/// Safe helper for headless/integration tests to set an `EnumParam` plain value.
pub fn set_enum_param<E: Enum + PartialEq + 'static>(param: &mut EnumParam<E>, value: E) {
    let name = param.name().to_string();
    *param = EnumParam::new(name, value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const VST3_MIDI_PARAMS_START: u32 = (1 << 31) - 130 * 16;
    const VST3_MIDI_PARAMS_END: u32 = 1 << 31;

    #[test]
    fn param_ids_and_vst3_hashes_are_unique() {
        let params = PhantomSpireParams::default();
        let map = params.param_map();
        assert!(!map.is_empty());

        let mut seen_ids = HashSet::with_capacity(map.len());
        let mut seen_hashes = HashSet::with_capacity(map.len());
        let mut bypass_count = 0usize;

        for (id, ptr, _group) in &map {
            assert!(
                seen_ids.insert(id.clone()),
                "Duplicate parameter ID found: {id}"
            );
            let h = hash_param_id(id);
            assert!(
                seen_hashes.insert(h),
                "31-bit hash collision for parameter ID '{id}' (hash {h})"
            );
            assert!(
                !(VST3_MIDI_PARAMS_START..VST3_MIDI_PARAMS_END).contains(&h),
                "Parameter ID '{id}' hash {h} collides with VST3 MIDI CC param range"
            );
            let flags = unsafe { ptr.flags() };
            if flags.contains(ParamFlags::BYPASS) {
                bypass_count += 1;
            }
        }

        assert!(bypass_count <= 1, "At most 1 BYPASS parameter allowed");
        assert_eq!(seen_ids.len(), map.len());
        assert_eq!(seen_hashes.len(), map.len());
    }

    #[test]
    fn param_groups_are_valid() {
        let params = PhantomSpireParams::default();
        for (id, _ptr, group) in params.param_map() {
            if !group.is_empty() {
                assert!(
                    !group.starts_with('/') && !group.ends_with('/') && !group.contains("//"),
                    "Invalid group path '{group}' on param '{id}'"
                );
                for segment in group.split('/') {
                    assert!(
                        !segment.is_empty(),
                        "Empty group segment in '{group}' on param '{id}'"
                    );
                }
            }
        }
    }

    #[test]
    fn param_ranges_defaults_and_formatters_are_pluginval_safe() {
        let params = PhantomSpireParams::default();
        assert_eq!(params.editor_state.size(), (1220, 760));

        for (id, ptr, _group) in params.param_map() {
            unsafe {
                let default_norm = ptr.default_normalized_value();
                assert!(
                    (0.0..=1.0).contains(&default_norm) && default_norm.is_finite(),
                    "Default normalized value out of [0, 1] for '{id}': {default_norm}"
                );
                for probe in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
                    let text = ptr.normalized_value_to_string(probe, true);
                    assert!(
                        !text.is_empty(),
                        "Empty formatted string at {probe} for '{id}'"
                    );
                    match ptr {
                        ParamPtr::FloatParam(p) => {
                            let plain = (*p).preview_plain(probe);
                            assert!(
                                plain.is_finite(),
                                "Non-finite plain value at {probe} for '{id}'"
                            );
                        }
                        ParamPtr::IntParam(p) => {
                            let _ = (*p).preview_plain(probe);
                        }
                        ParamPtr::BoolParam(p) => {
                            let _ = (*p).preview_plain(probe);
                        }
                        ParamPtr::EnumParam(p) => {
                            let idx = (*p).preview_plain(probe);
                            assert!(
                                (idx as usize) < (*p).len(),
                                "Enum index {idx} out of bounds for '{id}'"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn persist_and_param_state_round_trip() {
        let params_a = PhantomSpireParams::default();
        {
            let mut extra = params_a.extra_state.lock().unwrap();
            extra.preset_name = String::from("Test Velvet");
            extra.preset_category = String::from("Pad");
            extra.stepper_steps[0][0] = 0.92;
            extra.stepper_steps[1][15] = 0.13;
            extra.stepper_ties = [0b1010_0001, 0b0000_1111];
            extra.arp_steps_enabled = 0xAA55;
            extra.arp_step_offsets[3] = 12;
            extra.arp_step_velocities[7] = 64;
            extra.mod_sources[0] = 5;
            extra.mod_targets[0] = 30;
            extra.microtuning_preset = 2;
            extra.microtuning_cents[4] = -13.7;
        }

        let serialized = params_a.serialize_fields();
        let json_str = serde_json::to_string(&serialized).expect("serialize fields map");
        let restored_map: BTreeMap<String, String> =
            serde_json::from_str(&json_str).expect("deserialize fields map");

        let params_b = PhantomSpireParams::default();
        params_b.deserialize_fields(&restored_map);

        let extra_a = params_a.extra_state.lock().unwrap().clone();
        let extra_b = params_b.extra_state.lock().unwrap().clone();
        assert_eq!(extra_a, extra_b);
        assert_eq!(params_b.editor_state.size(), (1220, 760));
    }
}
