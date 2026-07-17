//! MegaLogViewer `.msl` datalog output (and, later, reading for the log
//! viewer). Tab-separated, CRLF line endings, three header lines (title /
//! column labels / units), columns and formats taken from the INI
//! `[Datalog]` section. Rows are flushed per write and fsync'd periodically
//! so an ignition-off power cut loses at most about a second of data.
