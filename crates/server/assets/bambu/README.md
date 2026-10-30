# Bambu Studio presets

What the exported project 3MF (`bambu.rs`) needs to open in Bambu Studio with its printer,
process and filament presets already chosen:

- `x1c-0.4-0.20-standard-10-filaments.json`: the `Metadata/project_settings.config` Bambu
  Studio 02.07.01.62 writes into a project: its stock presets for the Bambu Lab X1 Carbon,
  0.4 mm nozzle, 0.20 mm Standard, with ten filament slots (Bambu PLA Matte, Galaxy, Sparkle and
  Metal). Taken from a project saved in Studio; no personal settings are in it. A project's own
  settings are cut from it slot by slot.
- `colours.json`: Bambu's colour names and hex codes for those filament presets, from Bambu's
  store pages, cross-checked against a Studio-saved project (the `_about` key says how).

These are configuration values for Bambu Studio to read, not code from it. Bambu Studio itself
is AGPL-3.0, and none of its source is in this repository.
