# Phantom Spire Project Instructions

## Mandatory Post-Change Verification Rule
Whenever you finish making any code, UI, DSP, preset, or build changes in this project, you **MUST** always:
1. Run the standalone 4-step verification script before reporting completion:
   ```powershell
   powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify_plugin.ps1
   ```
   *(Pass `-AllTabs` when UI changes span `FX & MASTER`, `MOD & ARP`, or the Preset Browser.)*
2. Open and visually inspect the generated `.png` screenshot(s) (`scripts\.artifacts\ui_screenshot.png`, or `%TEMP%\phantom_spire_visual\ui_screenshot.png`) using `read_file` to verify that no knobs overlap, no labels or values are clipped/cut off, and the Phantom Voicer visual design renders cleanly before telling the user you are done.
