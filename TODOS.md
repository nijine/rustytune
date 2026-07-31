# TODOs

Known gaps and planned work. The INI-coverage items below come from an audit
of `fixtures/speeduino202501_7.ini` against the parser (`crates/ts-ini`) and
the server/UI that consumes it. The parser emits **no warnings** on the
fixture, so nothing here is a mis-parse — these are fields that are skipped,
parsed-but-unused, or not surfaced in the UI.

## General

- Add Windows build support & matching CI workflow (see the Windows note in
  [README.md](README.md) — serial comms are modeled \*nix-only today).
- Add autotune functionality (see `[VeAnalyze]` / `[WueAnalyze]` below).

## INI sections not parsed

`crates/ts-ini/src/lib.rs` dispatches per section and falls through to
`_ => {}` for these. Line numbers are into the reference fixture.

- **`[SettingContextHelp]`** (line 2138, 378 entries) — per-field help text.
  Only dialog-level `topicHelp` (a wiki URL) is served today
  (`crates/server/src/api.rs:1303`), so no field tooltips anywhere.
- **`[ReferenceTables]`** (line 5848, 72 lines) — sensor calibration tables
  (CLT/IAT thermistor, O2, MAP), plus `tableWriteCommand` and
  `tableBlockingFactor`. There is no calibration support anywhere in the
  server or `ecu-proto`; this is a whole missing feature, not a partial one.
- **`[LoggerDefinition]`** (line 5745, 67 lines) — tooth and composite trigger
  loggers (`H`/`h`/`J`/`j` commands, `recordDef`/`recordField`).
- **`[ControllerCommands]`** (line 4541, 60 commands) — the `E\x..` command
  strings behind every `commandButton` (output tests, resets, calibration
  triggers).
- **`[VeAnalyze]`** (line 5941) and **`[WueAnalyze]`** (line 5968) — autotune
  maps and filters.
- **`[Tools]`** (line 5933) — `veTableGenerator`, `afrTableGenerator`.
- **`[EventTriggers]`** (line 1554) — `triggeredPageRefresh = 1,
  { vssRefresh > 0 }`; page 1 is never re-read when the ECU updates VSS
  values.
- **`[TunerStudio]`** (line 11) — `iniSpecVersion = 3.78`.

## Keys unhandled inside parsed sections

- `[Constants]`: `restrictSquirtRelationship` (line 267) and
  `readSdCompressed` (line 274) fall into `ConstantsHeader::misc`.
- `[CurveEditor]`: `size` (×12), `gauge` (×5), `lineLabel` (×2), `topicHelp`
  (×1) fall into `CurveDef::misc`.
- All three `misc` maps (`ConstantsHeader::misc`, `TableDef::misc`,
  `CurveDef::misc`) are write-only — nothing in `server` or `tune-model`
  reads them.

## `[UserDefined]` elements discarded at parse time

`crates/ts-ini/src/sections.rs` drops these element types outright:
`commandButton` (×60), `settingOption` (×45), `indicator` (×6),
`settingSelector` (×5), `graphLine` (×4), `gauge` (×4), `text` (×4),
`liveGraph` (×1), `indicatorPanel` (×1), `help` (×1), `webHelp` (×1).

`commandButton` is the significant one: it is the entire output-test,
calibrate, and reset surface, and it depends on `[ControllerCommands]` above.

## Parsed into the model but never consumed

- **`setting_groups`** (6 groups, 11 options) — no reader at all. The
  preprocessor symbol profile (`CELSIUS`, `LAMBDA`, `mcu_*`,
  `enablehardware_test`, ...) is CLI-only via `--symbols`
  (`crates/server/src/main.rs`), with no UI to choose one. Note that
  `enablehardware_test` gates the whole Hardware Testing menu (fixture line
  2126).
- **`controller_priority`** (8 entries: `vssPulsesPerKm`, `vssRatio1..6`,
  `bootloaderCaps`) — parsed, never read. These are the constants TunerStudio
  re-polls because the controller changes them on its own.
- **`TableDef::grid_height` / `grid_orient` / `up_down_label` / `topic_help`**
  — parsed, never serialized; only `xy_labels` reaches the UI.
- **`CurveDef::y_bins`** — the curve endpoint takes `.first()` only, so
  multi-trace curves render a single line. Affects `warmup_analyzer_curve`
  (2 `yBins`), the only multi-trace curve in the fixture.

## Menu targets that resolve to nothing

22 menu/panel targets resolve to nothing servable. 14 are the `*Map` 3D views
(intentionally unsupported, and documented as such). The remaining 8 are
real gaps:

- `std_tpscal` — TPS calibration (orphans the `tpsMin` / `tpsMax` constants).
- `std_ms3SdConsole` — onboard SD logging (orphans 10 `onboard_log_*`
  constants).
- `std_ms2gentherm` / `std_ms2geno2` — thermistor and O2 calibration; these
  need `[ReferenceTables]` above.
- `std_ms3Rtc` — realtime clock setup.
- `std_realtime`, `helpGeneral`, `protectIndicatorPanel`.

Only `std_injection` has a hand-rolled substitute today
(`crates/server/src/api.rs`).

### Unreachable constants

64 of 817 constants are unreachable from any dialog, table, or curve. Roughly
25 are genuine `unused*` INI padding. The real orphans cluster around the
missing targets above, plus CAN output configuration
(`canoutput_param_group`, `candID`, `firstDataIn` / `firstTarget` /
`secondDataIn` / `secondTarget`).

35 dialogs are unreachable from any menu, but most of that is faithful to the
INI: `Canout_config` and `knock_windows` are commented out in the fixture
itself (lines 2086, 2679), and the `outputtest*` / `stm32cmd` cluster sits
behind `#if enablehardware_test`.

## Bugs

- **TunerStudio label markers render literally.** TS treats a leading `!` as
  critical/red and `#` as a section header; both are passed through verbatim,
  so roughly 20 settings rows read `#Note`, `!This is a critical setting!`,
  `#Fast Idle`, and so on (`web/src/components/SettingsView.tsx` renders the
  label raw).
- **Unquoted `!"..."` labels keep their quotes.** The INI's `field = !"text"`
  form (fixture line 4197 and 6 others) lexes to a `Bare` token that retains
  its quote characters, because `crates/ts-ini/src/lex.rs` only strips quotes
  when the piece *starts* with `"`. Visible today as
  `!"No PWM Fan available on MCU"` in the fan settings dialog.
