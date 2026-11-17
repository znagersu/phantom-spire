# Phantom Spire Visual QA & Agent-as-Judge Rubric

## Reference Benchmarks
- **Visual Style Reference (Pic 2)**: `Phantom Voicer` (`C:/Users/endio/.gemini/antigravity/brain/48de30d3-0701-4d45-89ed-a8e1bf7dde1e/.user_uploaded/media_1794950883239_46cd177c.jpg` and `C:\Users\endio\.gemini\antigravity\scratch\phantom-voicer\src\editor.rs`)
- **Synth Layout & Feature Reference (Pic 1)**: `Reveal Sound Spire` (`C:/Users/endio/.gemini/antigravity/brain/48de30d3-0701-4d45-89ed-a8e1bf7dde1e/.user_uploaded/media_1794950790446_78240997.jpg`)
- **Target Window Size**: `1220 x 760` (DPI scale 1.0)
- **Required Captured Views**:
  1. `synth.png` (`SYNTH` tab: Left Crimson Command Zone + OSC 1–4, Unison/Mix, Dual Filter 1–2, Mod Bays ENV/LFO/STP/ARP with `>= 8px` bottom fader margin + Bottom 49-key Live Monitor)
  2. `fx_master.png` (`FX & MASTER` tab: 5-unit FX Rack `SHP`, `PHS`, `CHR`, `DEL`, `REV` with `MUTE` + Master `X-Comp`, `Warm`/`Soft`/`Boost`, 3-Band `EQ` curve & controls)
  3. `mod_arp.png` (`MOD & ARP` tab: 15-slot Mod Matrix, Stepper 1–2 16-step grids, Arpeggiator pattern & clock controls)
  4. `presets.png` (`PRESETS` browser overlay: Bank filter `ALL/FACTORY/USER/★ FAV`, Category sidebar with counts, Search bar, scrollable preset list w/ category pills + author + favorite star, `SAVE AS`/`DELETE`/`INIT`/`UNDO`/`REDO`)

## Mandatory Hard-Fail Gates (Any Violation = Immediate FAIL)
1. **Zero Blank / Solid-Color Renders**: Whole image and all 4 quadrants must have luminance `stddev > 20`, exact `1220x760` client resolution.
2. **Zero Generic Unstyled Widgets**: No stock rounded gray `egui::ComboBox`, default `egui::Slider`, default `egui::Button`, or default gray frames anywhere on screen.
3. **Zero Clipped or Overlapping Text/Controls**: Every label, knob caption, numeric readout, dropdown text, and header badge must be fully readable without overlap, cutoff, or illegible contrast.
4. **Functional Tab Switching**: `synth.png`, `fx_master.png`, and `mod_arp.png` must show genuinely distinct right-panel contents (pairwise pixel diff `>= 8%`).

## Scored Evaluation Categories (100 Points Total; Pass Threshold >= 90/100)

| # | Category | Weight | Criteria (Compared Side-by-Side Against Pic 2 & Pic 1) |
|---|----------|-------:|--------------------------------------------------------|
| 1 | **P5 Color Palette Fidelity** | 12 | Exact Persona 5 / Phantom Voicer palette: deep obsidian `#0B0B0E`/`#0C0C10` canvas, crimson `#E6001A`/`#8C0412` left command zone, crisp `#F8F8FA` white, `#000000` hard comic shadows, and P5 accents `#F7B801` (gold), `#1CE8FF` (cyan), `#FF3EA5` (magenta). |
| 2 | **Typography & Ransom/3D Headers** | 12 | Bold P5 sticker/cutout 3D typography (`Impact`/`Arial Black`/`Georgia Bold`/`Comic Bold` style), tilted alternating-block ransom logo (`PHANTOM SPIRE`), crisp dark-backed cutout section badges (`01 // ...` or Spire section tags). |
| 3 | **Left Crimson Command Zone (~270px)** | 10 | Crimson vertical command panel with halftone dot texture, diagonal comic cut accent, logo banner, Preset Browser (`<`/`>`, Menu, Init, Undo/Redo), Macros 1–4, Microtuning/Transpose, Pitch/Mod wheels, Glide/Bender/Drift, and bottom status/updater pill (`CLEANSE`/`UP TO DATE` style). |
| 4 | **Right Starburst Canvas & Cutout Cards** | 10 | Starburst/spiky geometric background on `#0B0B0E` obsidian canvas, sharp crisp white-bordered cutout module cards (`#0C0C10`/`#131318`) with offset hard black drop shadows matching Pic 2's card geometry. |
| 5 | **Custom 270° Knobs & Controls** | 12 | Custom P5 270° colored-arc rotary knobs (crimson/gold/cyan/white arcs, crisp pointer needle, cutout label + colored value readout), custom P5 horizontal/vertical sliders (sharp red fill bars like Pic 2 `Out Gain`/`Strum ms`), diamond/angled LED toggles & FX pills. |
| 6 | **Custom Dropdowns & Visual Plots** | 10 | Custom P5 sharp-bordered dropdown selectors (no generic egui popups/frames), crisp high-contrast plots for Oscillator Waveform, Envelope 6-stage curve, EQ frequency response, Stepper 16-step bars, and Arp pattern grid. |
| 7 | **Top Header Bar & ACTIVE NOTE Trapezoid** | 8 | Tilted P5 tab headers (`SYNTH`, `FX & MASTER`, `MOD & ARP`), mode/voices/MIDI learn/LCD parameter readout, L/R peak meter, and signature top-right white/red skewed `ACTIVE NOTE` trapezoid card (`READY` / active note). |
| 8 | **49-Key Live Monitor Piano Strip** | 8 | Full-width bottom `Live Monitor` cutout header with crisp black-and-white 49-key playable piano strip, clear active-note highlight colors, matching Pic 2's bottom strip proportions. |
| 9 | **Spire Feature Completeness & Ergonomics** | 10 | All Pic 1 Spire controls cleanly organized across the 3 tabs without clutter: OSC 1–4 + modes/unison/mix, Dual Filter 1–2 (PAR/LINK), ENV 1–4, LFO 1–4, STP 1–2, ARP, 15-slot MTRX, SHP/PHS/CHR/DEL/REV + MUTE, Master X-Comp/Warm-Soft-Boost/EQ. |
| 10 | **Overall Polish, Alignment & Zero Visual Defects** | 8 | Balanced margins, consistent grid alignment, crisp contrast at 1220x760, zero clipped labels, zero overlapping hitboxes, zero visual glitches or artifacts. |

## Judge Output Protocol
When inspecting captured PNGs via `read_file`:
1. List quantitative pixel metrics from `analyze_ui.ps1` for all 3 tabs.
2. Score each of the 10 categories explicitly with specific visual observations from `synth.png`, `fx_master.png`, and `mod_arp.png`.
3. Report any defect (even minor clipping or generic widget) directly to `@ui` with exact tab, coordinate region, and expected fix.
4. PASS only if **all 4 Hard-Fail Gates pass** AND **Total Score >= 90/100**.
