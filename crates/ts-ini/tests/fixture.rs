//! Golden assertions against the real Speeduino 202405-dev INI.

use std::collections::HashSet;

use ts_ini::{
    ConstantClass, DataType, DialogItem, IniDef, MenuItem, NumOrExpr, OutputChannel, Shape,
    SymbolSource, Value,
};

const FIXTURE: &str = include_str!("../../../fixtures/speeduino202405_dev.ini");

fn parse_default() -> &'static IniDef {
    static DEF: std::sync::OnceLock<IniDef> = std::sync::OnceLock::new();
    DEF.get_or_init(|| ts_ini::parse(FIXTURE).expect("fixture must parse with default symbols"))
}

fn parse_with(symbols: &[&str]) -> IniDef {
    let set: HashSet<String> = symbols.iter().map(|s| s.to_string()).collect();
    ts_ini::parse_with_symbols(FIXTURE, &set).expect("fixture must parse")
}

#[test]
fn parses_under_all_symbol_profiles() {
    for profile in [
        &[][..],
        &["CELSIUS"][..],
        &["LAMBDA"][..],
        &["CELSIUS", "LAMBDA"][..],
        &["mcu_stm32"][..],
        &["mcu_teensy"][..],
        &["COMMS_COMPAT"][..],
        &["MSDROID_COMPAT"][..],
        &["pressure_bar", "resetcontrol_adv"][..],
    ] {
        let def = parse_with(profile);
        assert_eq!(def.header.n_pages, 15, "profile {profile:?}");
    }
}

#[test]
fn no_warnings_on_the_fixture() {
    let def = parse_default();
    assert_eq!(def.warnings, Vec::<String>::new());
}

#[test]
fn megatune_header() {
    let def = parse_default();
    assert_eq!(def.signature, "speeduino 202405-dev");
    assert_eq!(def.query_command, "Q");
    assert_eq!(def.version_info, "S");
}

#[test]
fn constants_header_golden() {
    let def = parse_default();
    let h = &def.header;
    assert_eq!(h.endianness, "little");
    assert_eq!(h.n_pages, 15);
    assert_eq!(
        h.page_sizes,
        vec![
            128, 288, 288, 128, 288, 128, 240, 384, 192, 192, 288, 192, 128, 288, 256
        ]
    );
    assert_eq!(h.page_sizes.len(), h.n_pages);
    assert_eq!(h.blocking_factor, Some(251)); // 121 only for stm32/compat
    assert_eq!(h.page_read_command[0], "p%2i%2o%2c");
    assert_eq!(h.page_chunk_write[7], "M%2i%2o%2c%v");
    assert_eq!(h.burn_command[0], "b%2i");
    assert_eq!(h.crc32_check_command[14], "d%2i");
    assert_eq!(h.page_identifiers[0], r"\$tsCanId\x01");
    assert_eq!(h.delay_after_port_open, Some(1000));
    assert!(h.ts_write_blocks);
    assert_eq!(h.page_activation_delay, Some(10));
    assert_eq!(h.table_crc_command.as_deref(), Some(r"k\$tsCanId%2i%2o%2c"));
}

#[test]
fn comms_compat_profile_changes_burn_and_blocking() {
    let def = parse_with(&["COMMS_COMPAT"]);
    assert_eq!(def.header.burn_command[0], "B%2i");
    assert_eq!(def.header.blocking_factor, Some(121));
}

#[test]
fn ve_table_constants_golden() {
    let def = parse_default();

    let ve = &def.constants["veTable"];
    assert_eq!(ve.page, Some(2));
    assert_eq!(ve.offset, Some(0));
    assert_eq!(ve.ty, DataType::U08);
    assert_eq!(ve.shape, Some(Shape::Array2D { x: 16, y: 16 }));
    assert_eq!(ve.byte_size(), 256);

    let rpm_bins = &def.constants["rpmBins"];
    assert_eq!(rpm_bins.page, Some(2));
    assert_eq!(rpm_bins.offset, Some(256));
    assert_eq!(rpm_bins.shape, Some(Shape::Array1D(16)));
    assert_eq!(rpm_bins.scale.literal(), Some(100.0));
}

#[test]
fn scalar_constant_golden() {
    let def = parse_default();

    let req_fuel = &def.constants["reqFuel"];
    assert_eq!(req_fuel.page, Some(1));
    assert_eq!(req_fuel.offset, Some(24));
    assert_eq!(req_fuel.class, ConstantClass::Scalar);
    assert_eq!(req_fuel.scale.literal(), Some(0.1));
    assert_eq!(
        req_fuel.digits.as_ref().and_then(|d| d.literal()),
        Some(1.0)
    );

    let n_cyl = &def.constants["nCylinders"];
    assert_eq!(n_cyl.class, ConstantClass::Bits);
    assert_eq!(n_cyl.offset, Some(36));
    assert_eq!(n_cyl.shape, Some(Shape::Bits { lo: 4, hi: 7 }));
    assert_eq!(n_cyl.labels[1], "1");
    assert_eq!(n_cyl.labels[0], "INVALID");
}

#[test]
fn last_offset_aliases_previous_constant() {
    let def = parse_default();
    // ego_min_lambda overlays ego_min_afr (same byte, lambda display).
    assert_eq!(def.constants["ego_min_afr"].offset, Some(8));
    assert_eq!(def.constants["ego_min_lambda"].offset, Some(8));
    assert_eq!(def.constants["ego_max_afr"].offset, Some(9));
    assert_eq!(def.constants["ego_max_lambda"].offset, Some(9));
    // afrTable is the AFR view of lambdaTable's bytes.
    assert_eq!(def.constants["lambdaTable"].offset, Some(0));
    assert_eq!(def.constants["afrTable"].offset, Some(0));
}

#[test]
fn every_constant_fits_its_page() {
    let def = parse_default();
    for c in def.constants.values() {
        let page = c.page.expect("page constant") as usize;
        let size = def.header.page_sizes[page - 1];
        let end = c.offset.unwrap() + c.byte_size();
        assert!(
            end <= size,
            "{} (page {page}, offset {:?}, {} bytes) exceeds page size {size}",
            c.name,
            c.offset,
            c.byte_size()
        );
    }
}

#[test]
fn output_channels_golden() {
    let def = parse_default();
    assert_eq!(def.och_block_size, 127);
    assert_eq!(def.och_get_command, r"r\$tsCanId\x30%2o%2c");

    let OutputChannel::Scalar {
        ty, offset, scale, ..
    } = &def.output_channels["rpm"]
    else {
        panic!("rpm should be a scalar channel");
    };
    assert_eq!(*ty, DataType::U16);
    assert_eq!(*offset, 14);
    assert_eq!(scale.literal(), Some(1.0));

    let OutputChannel::Scalar { offset, scale, .. } = &def.output_channels["tps"] else {
        panic!("tps should be a scalar channel");
    };
    assert_eq!(*offset, 25);
    assert_eq!(scale.literal(), Some(0.5));

    let OutputChannel::Bits { offset, lo, hi, .. } = &def.output_channels["running"] else {
        panic!("running should be a bits channel");
    };
    assert_eq!((*offset, *lo, *hi), (2, 0, 0));

    // Channels must fit inside the realtime data block.
    for (name, ch) in &def.output_channels {
        let end = match ch {
            OutputChannel::Scalar { ty, offset, .. } => offset + ty.size(),
            OutputChannel::Bits { ty, offset, .. } => offset + ty.size(),
            OutputChannel::Derived { .. } => 0,
        };
        assert!(end <= def.och_block_size, "{name} exceeds ochBlockSize");
    }
}

#[test]
fn derived_channels_evaluate() {
    struct Telemetry;
    impl SymbolSource for Telemetry {
        fn value(&self, name: &str) -> Option<Value> {
            Some(Value::Num(match name {
                "coolantRaw" => 130.0,
                "afr" => 14.7,
                "stoich" => 14.7,
                _ => return None,
            }))
        }
    }

    // Default profile is Fahrenheit: (raw - 40) * 1.8 + 32.
    let def = parse_default();
    let OutputChannel::Derived { expr: coolant, .. } = &def.output_channels["coolant"] else {
        panic!("coolant should be derived");
    };
    assert_eq!(coolant.eval(&Telemetry).unwrap(), Value::Num(194.0));

    let OutputChannel::Derived { expr: lambda, .. } = &def.output_channels["lambda"] else {
        panic!("lambda should be derived");
    };
    assert_eq!(lambda.eval(&Telemetry).unwrap(), Value::Num(1.0));

    // CELSIUS profile: plain offset.
    let def_c = parse_with(&["CELSIUS"]);
    let OutputChannel::Derived {
        expr: coolant_c, ..
    } = &def_c.output_channels["coolant"]
    else {
        panic!("coolant should be derived");
    };
    assert_eq!(coolant_c.eval(&Telemetry).unwrap(), Value::Num(90.0));
}

#[test]
fn tables_golden() {
    let def = parse_default();
    let ve = &def.tables["veTable1Tbl"];
    assert_eq!(ve.title, "VE Table");
    assert_eq!(ve.page, 2);
    assert_eq!(ve.x_bins, ("rpmBins".to_string(), Some("rpm".to_string())));
    assert_eq!(ve.y_bins.0, "fuelLoadBins");
    assert_eq!(ve.z_bins, "veTable");

    let spark = &def.tables["sparkTbl"];
    assert_eq!(spark.page, 3);
    assert_eq!(spark.z_bins, "advTable1");

    let afr = &def.tables["afrTable1Tbl"];
    assert_eq!(afr.page, 5);
    assert_eq!(afr.z_bins, "afrTable");

    // Every table's bins must reference real constants. (Bins constants may
    // live on a different page than the table's `page` attribute — e.g.
    // boostDCLupTbl is page 7 with its data on page 15 — so resolve
    // constants by name, never by the table's page.)
    for (id, t) in &def.tables {
        for bins in [&t.x_bins.0, &t.y_bins.0, &t.z_bins] {
            assert!(
                def.constants.contains_key(bins),
                "table {id}: unknown bins constant `{bins}`"
            );
        }
    }
}

#[test]
fn curves_reference_real_constants() {
    let def = parse_default();
    assert!(!def.curves.is_empty());
    let dwell = &def.curves["dwell_correction_curve"];
    assert_eq!(dwell.x_bins.0, "brvBins");
    assert_eq!(dwell.y_bins, vec!["dwellRates".to_string()]);

    // Curves may edit page constants or PcVariables (e.g. wueAFR).
    let known =
        |name: &str| def.constants.contains_key(name) || def.pc_variables.contains_key(name);
    for (id, c) in &def.curves {
        assert!(
            known(&c.x_bins.0),
            "curve {id}: unknown xBins `{}`",
            c.x_bins.0
        );
        for y in &c.y_bins {
            assert!(known(y), "curve {id}: unknown yBins `{y}`");
        }
    }
}

#[test]
fn gauges_golden() {
    let def = parse_default();
    let tach = &def.gauges["tachometer"];
    assert_eq!(tach.channel, "rpm");
    assert_eq!(tach.lo.literal(), Some(0.0));
    // hi is {rpmhigh}: an expression resolved against PcVariables.
    assert!(tach.hi.literal().is_none());

    struct Pc;
    impl SymbolSource for Pc {
        fn value(&self, name: &str) -> Option<Value> {
            (name == "rpmhigh").then_some(Value::Num(8000.0))
        }
    }
    assert_eq!(tach.hi.eval(&Pc).unwrap(), 8000.0);

    // Front page wires up gauges that exist.
    assert_eq!(def.front_page.gauges[0], "tachometer");
    for g in &def.front_page.gauges {
        assert!(
            def.gauges.contains_key(g),
            "front page gauge `{g}` undefined"
        );
    }
    assert!(!def.front_page.indicators.is_empty());
}

#[test]
fn pc_variables_golden() {
    let def = parse_default();
    let rpmhigh = &def.pc_variables["rpmhigh"];
    assert_eq!(rpmhigh.offset, None);
    assert_eq!(rpmhigh.ty, DataType::U16);

    let alias = &def.pc_variables["AUXin00Alias"];
    assert_eq!(alias.class, ConstantClass::String);
    assert_eq!(alias.string_len, Some(20));

    let board = &def.pc_variables["boardFuelOutputs"];
    assert!(board.no_msq_save);
}

#[test]
fn constants_extensions_golden() {
    let def = parse_default();
    assert!(def.requires_power_cycle.iter().any(|n| n == "nCylinders"));
    assert!(
        def.default_values
            .iter()
            .any(|(n, v)| n == "pinLayout" && v == "1")
    );
    // Every requiresPowerCycle name should exist as a constant.
    for name in &def.requires_power_cycle {
        assert!(
            def.constants.contains_key(name),
            "requiresPowerCycle references unknown constant `{name}`"
        );
    }
}

#[test]
fn datalog_golden() {
    let def = parse_default();
    let time = &def.datalog[0];
    assert_eq!(time.channel, "time");
    assert_eq!(time.format, "%.3f");

    let rpm = def.datalog.iter().find(|e| e.channel == "rpm").unwrap();
    assert_eq!(rpm.label, "RPM");

    let vvt = def
        .datalog
        .iter()
        .find(|e| e.channel == "vvt1Angle")
        .unwrap();
    assert!(vvt.condition.is_some());

    // Datalog entries reference output channels or, occasionally, tune
    // constants directly (flexBoostAdj).
    for e in &def.datalog {
        assert!(
            def.output_channels.contains_key(&e.channel) || def.constants.contains_key(&e.channel),
            "datalog entry `{}` has no output channel or constant",
            e.channel
        );
    }
}

#[test]
fn scale_expressions_evaluate() {
    let def = parse_with(&["LAMBDA"]);
    // wueAFR (PcVariables) under LAMBDA: scale = {0.1 / stoich}.
    let wue = &def.pc_variables["wueAFR"];
    let NumOrExpr::Expr(_) = &wue.scale else {
        panic!("wueAFR scale should be an expression under LAMBDA");
    };

    struct Stoich;
    impl SymbolSource for Stoich {
        fn value(&self, name: &str) -> Option<Value> {
            (name == "stoich").then_some(Value::Num(14.7))
        }
    }
    let v = wue.scale.eval(&Stoich).unwrap();
    assert!((v - 0.1 / 14.7).abs() < 1e-12);
}

#[test]
fn setting_groups_parsed() {
    let def = parse_default();
    let mcu = def
        .setting_groups
        .iter()
        .find(|g| g.keyword == "mcu")
        .expect("mcu setting group");
    assert_eq!(mcu.options.len(), 3);
    assert_eq!(mcu.options[1].0, "mcu_teensy");
}

#[test]
fn menus_and_dialogs_golden() {
    let def = parse_default();

    let titles: Vec<&str> = def.menus.iter().map(|m| m.title.as_str()).collect();
    assert!(titles.contains(&"Tuning"), "menus: {titles:?}");
    assert!(titles.contains(&"Startup/Idle"));
    assert!(titles.contains(&"Accessories"));

    // "&Tuning" has its mnemonic stripped and links Acceleration Enrichment.
    let tuning = def.menus.iter().find(|m| m.title == "Tuning").unwrap();
    let accel = tuning
        .items
        .iter()
        .find_map(|i| match i {
            MenuItem::Entry(e) if e.target == "accelEnrichments" => Some(e),
            _ => None,
        })
        .expect("accelEnrichments entry");
    assert_eq!(accel.label, "Acceleration Enrichment");

    // groupMenu "Engine Protection" collects its children.
    let protection = tuning
        .items
        .iter()
        .find_map(|i| match i {
            MenuItem::Group { label, children } if label == "Engine Protection" => Some(children),
            _ => None,
        })
        .expect("Engine Protection group");
    assert_eq!(protection.len(), 5);
    assert!(
        protection[1].enable.is_some(),
        "Rev Limiters gated on engineProtectType"
    );

    // idleSettings: field with combo constant, enable-gated field, panels.
    let idle = &def.dialogs["idleSettings"];
    assert_eq!(idle.title, "Idle Settings");
    let mut fields = idle.items.iter();
    let has = |target: &str, d: &ts_ini::DialogDef| {
        d.items
            .iter()
            .any(|i| matches!(i, DialogItem::Panel { target: t, .. } if t == target))
    };
    assert!(has("pwm_idle", idle) && has("stepper_idle", idle) && has("closedloop_idle", idle));
    assert!(fields.any(|i| matches!(
        i,
        DialogItem::Field { label, constant: Some(c), .. }
            if label == "Idle control type" && c == "iacAlgorithm"
    )));

    // TunerStudio permits a condition without a comma after the constant.
    // Keep the trigger edge wired to its real constant and condition.
    let trigger_edge = def.dialogs["triggerSettings"]
        .items
        .iter()
        .find_map(|item| match item {
            DialogItem::Field {
                label,
                constant,
                enable,
                ..
            } if label == "Trigger edge" => Some((constant, enable)),
            _ => None,
        })
        .expect("Trigger edge field");
    assert_eq!(trigger_edge.0.as_deref(), Some("TrigEdge"));
    assert!(trigger_edge.1.is_some());

    // field = label, name, {}, { visible } keeps the second slot as visible.
    let aux = &def.dialogs["Auxinput_pin_selection"];
    let vis_gated = aux
        .items
        .iter()
        .filter(|i| {
            matches!(
                i,
                DialogItem::Field {
                    enable: None,
                    visible: Some(_),
                    ..
                }
            )
        })
        .count();
    assert!(
        vis_gated >= 16,
        "visibility-gated aux fields, got {vis_gated}"
    );

    // Header rows ("#...") have no constant.
    assert!(idle.items.iter().any(|i| matches!(
        i,
        DialogItem::Field { label, constant: None, .. } if label.starts_with('#')
    )));

    assert!(def.dialogs["idleSettings"].topic_help.is_some());
}
