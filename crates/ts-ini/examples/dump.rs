//! Parse a TunerStudio INI and print a summary of the model.
//!
//! Usage: cargo run -p ts-ini --example dump -- <file.ini> [SYMBOL ...]

use std::collections::HashSet;

use ts_ini::{ConstantClass, OutputChannel};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!("usage: dump <file.ini> [SYMBOL ...]");
        std::process::exit(2);
    });
    let symbols: HashSet<String> = args.collect();

    let src = std::fs::read_to_string(&path).expect("read ini");
    let started = std::time::Instant::now();
    let def = match ts_ini::parse_with_symbols(&src, &symbols) {
        Ok(def) => def,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };
    let elapsed = started.elapsed();

    println!("signature      : {}", def.signature);
    println!(
        "pages          : {} {:?}",
        def.header.n_pages, def.header.page_sizes
    );
    println!(
        "blockingFactor : {:?}   tsWriteBlocks: {}",
        def.header.blocking_factor, def.header.ts_write_blocks
    );

    let mut by_class = [0usize; 4];
    for c in def.constants.values() {
        by_class[match c.class {
            ConstantClass::Scalar => 0,
            ConstantClass::Bits => 1,
            ConstantClass::Array => 2,
            ConstantClass::String => 3,
        }] += 1;
    }
    println!(
        "constants      : {} (scalar {}, bits {}, array {}, string {})",
        def.constants.len(),
        by_class[0],
        by_class[1],
        by_class[2],
        by_class[3]
    );
    println!("pcVariables    : {}", def.pc_variables.len());

    let derived = def
        .output_channels
        .values()
        .filter(|c| matches!(c, OutputChannel::Derived { .. }))
        .count();
    println!(
        "outputChannels : {} ({} derived), ochBlockSize {}",
        def.output_channels.len(),
        derived,
        def.och_block_size
    );

    println!("tables         : {}", def.tables.len());
    for t in def.tables.values() {
        println!(
            "    {:24} page {:2}  {} / {} / {}",
            t.title, t.page, t.x_bins.0, t.y_bins.0, t.z_bins
        );
    }
    println!("curves         : {}", def.curves.len());
    println!("gauges         : {}", def.gauges.len());
    println!(
        "front page     : {} gauges, {} indicators",
        def.front_page.gauges.len(),
        def.front_page.indicators.len()
    );
    println!("datalog        : {} entries", def.datalog.len());
    println!(
        "extensions     : {} requiresPowerCycle, {} defaultValue, {} controllerPriority",
        def.requires_power_cycle.len(),
        def.default_values.len(),
        def.controller_priority.len()
    );

    if !def.warnings.is_empty() {
        println!("\nwarnings ({}):", def.warnings.len());
        for w in &def.warnings {
            println!("    {w}");
        }
    }
    println!("\nparsed in {elapsed:.2?}");
}
