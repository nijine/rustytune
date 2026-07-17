//! In-memory tune state.
//!
//! Pages are raw byte buffers sized from the INI; typed access to constants,
//! tables, and curves decodes through the ts-ini definition model. Each page
//! keeps three copies — local edits, the ECU-RAM shadow, and the burned
//! (EEPROM) shadow — whose diffs drive the unsent-write queue and the Burn
//! indicator. Also owns .msq (TunerStudio tune file) read/write.
