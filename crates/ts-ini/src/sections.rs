//! Per-section parsers turning preprocessed lines into the model.

use indexmap::IndexMap;

use crate::Error;
use crate::expr::{self, Expr};
use crate::lex::{Shape, Token, parse_shape, split_kv, tokenize};
use crate::model::*;
use crate::preprocess::Line;

pub struct Ctx<'a> {
    pub def: &'a mut IniDef,
}

impl Ctx<'_> {
    fn warn(&mut self, num: u32, msg: impl std::fmt::Display) {
        self.def.warnings.push(format!("line {num}: {msg}"));
    }
}

fn kv<'l>(line: &'l Line, ctx: &mut Ctx) -> Option<(&'l str, Vec<Token>)> {
    let Some((key, rest)) = split_kv(&line.text) else {
        ctx.warn(line.num, format!("expected `key = ...`: {}", line.text));
        return None;
    };
    match tokenize(rest, line.num) {
        Ok(tokens) => Some((key, tokens)),
        Err(e) => {
            ctx.warn(line.num, e);
            None
        }
    }
}

fn str_tok(tokens: &[Token], idx: usize, num: u32) -> Result<String, Error> {
    match tokens.get(idx) {
        Some(t) => Ok(t.as_str().to_string()),
        None => Err(Error::at(num, format!("missing value #{}", idx + 1))),
    }
}

fn num_or_expr(tokens: &[Token], idx: usize, num: u32) -> Result<NumOrExpr, Error> {
    match tokens.get(idx) {
        Some(Token::Expr(src)) => Ok(NumOrExpr::Expr(expr::parse(src, num)?)),
        Some(t) => Ok(NumOrExpr::Num(t.number(num)?)),
        None => Err(Error::at(
            num,
            format!("missing numeric value #{}", idx + 1),
        )),
    }
}

// ---------------------------------------------------------------- MegaTune

pub fn megatune(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        match key {
            "MTversion" => {
                ctx.def.mt_version = tokens.first().and_then(|t| t.number(line.num).ok())
            }
            "queryCommand" => ctx.def.query_command = str_tok(&tokens, 0, line.num)?,
            "signature" => ctx.def.signature = str_tok(&tokens, 0, line.num)?,
            "versionInfo" => ctx.def.version_info = str_tok(&tokens, 0, line.num)?,
            _ => ctx.warn(line.num, format!("unknown [MegaTune] key `{key}`")),
        }
    }
    Ok(())
}

// ----------------------------------------------------------- SettingGroups

pub fn setting_groups(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        match key {
            "settingGroup" => {
                ctx.def.setting_groups.push(SettingGroup {
                    keyword: str_tok(&tokens, 0, line.num)?,
                    display_name: str_tok(&tokens, 1, line.num).unwrap_or_default(),
                    options: Vec::new(),
                });
            }
            "settingOption" => {
                let opt = (
                    str_tok(&tokens, 0, line.num)?,
                    str_tok(&tokens, 1, line.num).unwrap_or_default(),
                );
                match ctx.def.setting_groups.last_mut() {
                    Some(g) => g.options.push(opt),
                    None => ctx.warn(line.num, "settingOption before settingGroup"),
                }
            }
            _ => ctx.warn(line.num, format!("unknown [SettingGroups] key `{key}`")),
        }
    }
    Ok(())
}

// ----------------------------------------------------- constant definitions

/// Classes that mark a `key = ...` line as a constant definition.
fn constant_class(tokens: &[Token]) -> Option<ConstantClass> {
    match tokens.first() {
        Some(Token::Bare(s)) => match s.as_str() {
            "scalar" => Some(ConstantClass::Scalar),
            "bits" => Some(ConstantClass::Bits),
            "array" => Some(ConstantClass::Array),
            "string" => Some(ConstantClass::String),
            _ => None,
        },
        _ => None,
    }
}

/// Parse a scalar/bits/array/string definition. `offset_ctx` is `Some` in
/// [Constants]/[OutputChannels] (field 3 is an offset, possibly the keyword
/// `lastOffset` = same offset as the previous constant) and `None` in
/// [PcVariables].
fn constant_def(
    name: &str,
    class: ConstantClass,
    tokens: &[Token],
    page: Option<u8>,
    offset_ctx: Option<u32>,
    line: u32,
) -> Result<ConstantDef, Error> {
    let ty = DataType::parse(tokens.get(1).map(|t| t.as_str()).unwrap_or(""), line)?;

    let mut def = ConstantDef {
        name: name.to_string(),
        class,
        ty,
        page,
        offset: None,
        shape: None,
        units: None,
        scale: NumOrExpr::Num(1.0),
        translate: NumOrExpr::Num(0.0),
        lo: None,
        hi: None,
        digits: None,
        labels: Vec::new(),
        string_len: None,
        no_msq_save: false,
        line,
    };

    // `string, ASCII, len` has no offset field even in [Constants]... except
    // it does when in a page; PcVariables omit it. Field layout is decided by
    // whether an offset is expected at index 2.
    let has_offset = offset_ctx.is_some();
    let mut idx = 2;
    if has_offset {
        let tok = tokens
            .get(idx)
            .ok_or_else(|| Error::at(line, "missing offset"))?;
        def.offset = Some(match tok {
            Token::Bare(s) if s == "lastOffset" => offset_ctx.unwrap(),
            t => t.number(line)? as u32,
        });
        idx += 1;
    }

    match class {
        ConstantClass::String => {
            def.string_len = Some(
                tokens
                    .get(idx)
                    .ok_or_else(|| Error::at(line, "missing string length"))?
                    .number(line)? as u32,
            );
            return Ok(def);
        }
        ConstantClass::Bits => {
            let shape = parse_shape(tokens.get(idx).map(|t| t.as_str()).unwrap_or(""), line)?;
            if !matches!(shape, Shape::Bits { .. }) {
                return Err(Error::at(line, "bits constant needs a [lo:hi] range"));
            }
            def.shape = Some(shape);
            def.labels = tokens[idx + 1..]
                .iter()
                .map(|t| t.as_str().to_string())
                .collect();
            return Ok(def);
        }
        ConstantClass::Array => {
            let shape = parse_shape(tokens.get(idx).map(|t| t.as_str()).unwrap_or(""), line)?;
            def.shape = Some(shape);
            idx += 1;
        }
        ConstantClass::Scalar => {}
    }

    // scalar/array tail: units, scale, translate, lo, hi, digits, flags
    if let Some(t) = tokens.get(idx) {
        def.units = Some(match t {
            Token::Expr(src) => StrOrExpr::Expr(expr::parse(src, line)?),
            t => StrOrExpr::Str(t.as_str().to_string()),
        });
    }
    if tokens.len() > idx + 1 {
        def.scale = num_or_expr(tokens, idx + 1, line)?;
    }
    if tokens.len() > idx + 2 {
        def.translate = num_or_expr(tokens, idx + 2, line)?;
    }
    if tokens.len() > idx + 3 {
        def.lo = Some(num_or_expr(tokens, idx + 3, line)?);
    }
    if tokens.len() > idx + 4 {
        def.hi = Some(num_or_expr(tokens, idx + 4, line)?);
    }
    for tok in tokens.iter().skip(idx + 5) {
        match tok {
            Token::Bare(s) if s == "noMsqSave" => def.no_msq_save = true,
            Token::Expr(src) if def.digits.is_none() => {
                def.digits = Some(NumOrExpr::Expr(expr::parse(src, line)?));
            }
            t if def.digits.is_none() => {
                def.digits = Some(NumOrExpr::Num(t.number(line)?));
            }
            _ => {} // flags we don't know yet
        }
    }
    Ok(def)
}

// ---------------------------------------------------------------- Constants

pub fn constants(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    let mut page: Option<u8> = None;
    let mut last_offset: Option<u32> = None;

    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };

        if key == "page" {
            page = Some(
                tokens
                    .first()
                    .ok_or_else(|| Error::at(line.num, "page = <n>"))?
                    .number(line.num)? as u8,
            );
            last_offset = None;
            continue;
        }

        if let Some(class) = constant_class(&tokens) {
            let cur_page = page.ok_or_else(|| Error::at(line.num, "constant before `page = n`"))?;
            let def = constant_def(
                key,
                class,
                &tokens,
                Some(cur_page),
                Some(last_offset.unwrap_or(0)),
                line.num,
            )?;
            last_offset = def.offset;
            ctx.def.constants.insert(key.to_string(), def);
            continue;
        }

        header_key(key, &tokens, line, ctx)?;
    }
    Ok(())
}

fn header_key(key: &str, tokens: &[Token], line: &Line, ctx: &mut Ctx) -> Result<(), Error> {
    let h = &mut ctx.def.header;
    let strings = |tokens: &[Token]| -> Vec<String> {
        tokens.iter().map(|t| t.as_str().to_string()).collect()
    };
    match key {
        "endianness" => h.endianness = str_tok(tokens, 0, line.num)?,
        "nPages" => h.n_pages = tokens[0].number(line.num)? as usize,
        "pageSize" => {
            h.page_sizes = tokens
                .iter()
                .map(|t| t.number(line.num).map(|n| n as u32))
                .collect::<Result<_, _>>()?;
        }
        "pageIdentifier" => h.page_identifiers = strings(tokens),
        "pageReadCommand" => h.page_read_command = strings(tokens),
        "pageValueWrite" => h.page_value_write = strings(tokens),
        "pageChunkWrite" => h.page_chunk_write = strings(tokens),
        "crc32CheckCommand" => h.crc32_check_command = strings(tokens),
        "burnCommand" => h.burn_command = strings(tokens),
        "tableCrcCommand" => h.table_crc_command = Some(str_tok(tokens, 0, line.num)?),
        "blockingFactor" => h.blocking_factor = Some(tokens[0].number(line.num)? as u32),
        "delayAfterPortOpen" => h.delay_after_port_open = Some(tokens[0].number(line.num)? as u32),
        "blockReadTimeout" => h.block_read_timeout = Some(tokens[0].number(line.num)? as u32),
        "tsWriteBlocks" => h.ts_write_blocks = tokens[0].as_str() == "on",
        "interWriteDelay" => h.inter_write_delay = Some(tokens[0].number(line.num)? as u32),
        "pageActivationDelay" => {
            h.page_activation_delay = Some(tokens[0].number(line.num)? as u32);
        }
        "messageEnvelopeFormat" => {
            h.message_envelope_format = Some(str_tok(tokens, 0, line.num)?);
        }
        _ => {
            let raw = tokens
                .iter()
                .map(|t| t.as_str().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            h.misc.insert(key.to_string(), raw);
        }
    }
    Ok(())
}

// -------------------------------------------------------------- PcVariables

pub fn pc_variables(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        match constant_class(&tokens) {
            Some(class) => {
                let def = constant_def(key, class, &tokens, None, None, line.num)?;
                ctx.def.pc_variables.insert(key.to_string(), def);
            }
            None => ctx.warn(
                line.num,
                format!("unrecognized [PcVariables] entry `{key}`"),
            ),
        }
    }
    Ok(())
}

// ------------------------------------------------------ ConstantsExtensions

pub fn constants_extensions(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        match key {
            "requiresPowerCycle" => {
                ctx.def
                    .requires_power_cycle
                    .push(str_tok(&tokens, 0, line.num)?);
            }
            "defaultValue" => {
                let name = str_tok(&tokens, 0, line.num)?;
                let rest = tokens[1..]
                    .iter()
                    .map(|t| t.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                ctx.def.default_values.push((name, rest));
            }
            "controllerPriority" => {
                ctx.def
                    .controller_priority
                    .push(str_tok(&tokens, 0, line.num)?);
            }
            other => ctx.warn(
                line.num,
                format!("unhandled [ConstantsExtensions] key `{other}`"),
            ),
        }
    }
    Ok(())
}

// ----------------------------------------------------------- OutputChannels

pub fn output_channels(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        match key {
            "ochGetCommand" => ctx.def.och_get_command = str_tok(&tokens, 0, line.num)?,
            "ochBlockSize" => ctx.def.och_block_size = tokens[0].number(line.num)? as u32,
            _ => {
                // Derived channel: `{ expr }` with optional trailing units.
                if let Some(Token::Expr(src)) = tokens.first() {
                    let e = expr::parse(src, line.num)?;
                    let units = tokens.get(1).map(|t| t.as_str().to_string());
                    ctx.def
                        .output_channels
                        .insert(key.to_string(), OutputChannel::Derived { expr: e, units });
                    continue;
                }
                let Some(class) = constant_class(&tokens) else {
                    ctx.warn(
                        line.num,
                        format!("unrecognized [OutputChannels] entry `{key}`"),
                    );
                    continue;
                };
                let def = constant_def(key, class, &tokens, None, Some(0), line.num)?;
                let ch = match class {
                    ConstantClass::Bits => {
                        let Some(Shape::Bits { lo, hi }) = def.shape else {
                            return Err(Error::at(line.num, "bits channel needs [lo:hi]"));
                        };
                        OutputChannel::Bits {
                            ty: def.ty,
                            offset: def.offset.unwrap_or(0),
                            lo,
                            hi,
                        }
                    }
                    _ => OutputChannel::Scalar {
                        ty: def.ty,
                        offset: def.offset.unwrap_or(0),
                        units: def.units,
                        scale: def.scale,
                        translate: def.translate,
                    },
                };
                ctx.def.output_channels.insert(key.to_string(), ch);
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------- TableEditor

pub fn table_editor(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    let mut current: Option<TableDef> = None;

    fn finish(current: &mut Option<TableDef>, tables: &mut IndexMap<String, TableDef>) {
        if let Some(t) = current.take() {
            tables.insert(t.id.clone(), t);
        }
    }

    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        if key == "table" {
            finish(&mut current, &mut ctx.def.tables);
            current = Some(TableDef {
                id: str_tok(&tokens, 0, line.num)?,
                map_id: str_tok(&tokens, 1, line.num)?,
                title: str_tok(&tokens, 2, line.num)?,
                page: tokens
                    .get(3)
                    .ok_or_else(|| Error::at(line.num, "table without page"))?
                    .number(line.num)? as u8,
                x_bins: (String::new(), None),
                y_bins: (String::new(), None),
                z_bins: String::new(),
                xy_labels: Vec::new(),
                grid_height: None,
                grid_orient: Vec::new(),
                up_down_label: Vec::new(),
                topic_help: None,
                misc: Vec::new(),
                line: line.num,
            });
            continue;
        }
        let Some(t) = current.as_mut() else {
            ctx.warn(
                line.num,
                format!("[TableEditor] key `{key}` outside a table"),
            );
            continue;
        };
        match key {
            "xBins" | "yBins" => {
                let pair = (
                    str_tok(&tokens, 0, line.num)?,
                    tokens.get(1).map(|t| t.as_str().to_string()),
                );
                if key == "xBins" {
                    t.x_bins = pair;
                } else {
                    t.y_bins = pair;
                }
            }
            "zBins" => t.z_bins = str_tok(&tokens, 0, line.num)?,
            "xyLabels" => {
                t.xy_labels = tokens.iter().map(|t| t.as_str().to_string()).collect();
            }
            "gridHeight" => t.grid_height = Some(tokens[0].number(line.num)?),
            "gridOrient" => {
                t.grid_orient = tokens
                    .iter()
                    .map(|tok| tok.number(line.num))
                    .collect::<Result<_, _>>()?;
            }
            "upDownLabel" => {
                t.up_down_label = tokens.iter().map(|t| t.as_str().to_string()).collect();
            }
            "topicHelp" => t.topic_help = Some(str_tok(&tokens, 0, line.num)?),
            other => t.misc.push((
                other.to_string(),
                tokens
                    .iter()
                    .map(|t| t.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            )),
        }
    }
    finish(&mut current, &mut ctx.def.tables);
    Ok(())
}

// ------------------------------------------------------------- CurveEditor

pub fn curve_editor(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    let mut current: Option<CurveDef> = None;

    fn finish(current: &mut Option<CurveDef>, curves: &mut IndexMap<String, CurveDef>) {
        if let Some(c) = current.take() {
            curves.insert(c.id.clone(), c);
        }
    }

    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        if key == "curve" {
            finish(&mut current, &mut ctx.def.curves);
            current = Some(CurveDef {
                id: str_tok(&tokens, 0, line.num)?,
                title: str_tok(&tokens, 1, line.num).unwrap_or_default(),
                column_label: Vec::new(),
                x_axis: Vec::new(),
                y_axis: Vec::new(),
                x_bins: (String::new(), None),
                y_bins: Vec::new(),
                misc: Vec::new(),
                line: line.num,
            });
            continue;
        }
        let Some(c) = current.as_mut() else {
            ctx.warn(
                line.num,
                format!("[CurveEditor] key `{key}` outside a curve"),
            );
            continue;
        };
        match key {
            "columnLabel" => {
                c.column_label = tokens.iter().map(|t| t.as_str().to_string()).collect();
            }
            "xAxis" | "yAxis" => {
                let axis = (0..tokens.len())
                    .map(|i| num_or_expr(&tokens, i, line.num))
                    .collect::<Result<Vec<_>, _>>()?;
                if key == "xAxis" {
                    c.x_axis = axis;
                } else {
                    c.y_axis = axis;
                }
            }
            "xBins" => {
                c.x_bins = (
                    str_tok(&tokens, 0, line.num)?,
                    tokens.get(1).map(|t| t.as_str().to_string()),
                );
            }
            "yBins" => c.y_bins.push(str_tok(&tokens, 0, line.num)?),
            other => c.misc.push((
                other.to_string(),
                tokens
                    .iter()
                    .map(|t| t.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            )),
        }
    }
    finish(&mut current, &mut ctx.def.curves);
    Ok(())
}

// ----------------------------------------------------- GaugeConfigurations

pub fn gauge_configurations(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    let mut category = String::new();
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        if key == "gaugeCategory" {
            category = str_tok(&tokens, 0, line.num)?;
            continue;
        }
        if tokens.len() < 9 {
            ctx.warn(line.num, format!("gauge `{key}` has too few fields"));
            continue;
        }
        let gauge = GaugeDef {
            name: key.to_string(),
            channel: str_tok(&tokens, 0, line.num)?,
            title: str_tok(&tokens, 1, line.num)?,
            units: str_tok(&tokens, 2, line.num)?,
            lo: num_or_expr(&tokens, 3, line.num)?,
            hi: num_or_expr(&tokens, 4, line.num)?,
            lo_danger: num_or_expr(&tokens, 5, line.num)?,
            lo_warn: num_or_expr(&tokens, 6, line.num)?,
            hi_warn: num_or_expr(&tokens, 7, line.num)?,
            hi_danger: num_or_expr(&tokens, 8, line.num)?,
            value_digits: tokens
                .get(9)
                .map(|t| t.number(line.num))
                .transpose()?
                .unwrap_or(0.0) as u8,
            label_digits: tokens
                .get(10)
                .map(|t| t.number(line.num))
                .transpose()?
                .unwrap_or(0.0) as u8,
            category: category.clone(),
        };
        ctx.def.gauges.insert(key.to_string(), gauge);
    }
    Ok(())
}

// ---------------------------------------------------------------- FrontPage

pub fn front_page(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    let mut gauges: Vec<(u32, String)> = Vec::new();
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        if let Some(n) = key
            .strip_prefix("gauge")
            .and_then(|n| n.parse::<u32>().ok())
        {
            gauges.push((n, str_tok(&tokens, 0, line.num)?));
            continue;
        }
        if key == "indicator" {
            let Some(Token::Expr(src)) = tokens.first() else {
                ctx.warn(line.num, "indicator without { condition }");
                continue;
            };
            ctx.def.front_page.indicators.push(Indicator {
                condition: expr::parse(src, line.num)?,
                off_label: str_tok(&tokens, 1, line.num)?,
                on_label: str_tok(&tokens, 2, line.num)?,
                colors: tokens[3..].iter().map(|t| t.as_str().to_string()).collect(),
            });
            continue;
        }
        ctx.warn(line.num, format!("unknown [FrontPage] key `{key}`"));
    }
    gauges.sort_by_key(|(n, _)| *n);
    ctx.def.front_page.gauges = gauges.into_iter().map(|(_, g)| g).collect();
    Ok(())
}

// ------------------------------------------------------------------ Datalog

pub fn datalog(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        if key != "entry" {
            ctx.warn(line.num, format!("unknown [Datalog] key `{key}`"));
            continue;
        }
        let ty = match tokens.get(2).map(|t| t.as_str()) {
            Some("int") => DatalogType::Int,
            Some("float") => DatalogType::Float,
            other => {
                ctx.warn(line.num, format!("unknown datalog type {other:?}"));
                continue;
            }
        };
        let condition = match tokens.get(4) {
            Some(Token::Expr(src)) => Some(expr::parse(src, line.num)?),
            _ => None,
        };
        ctx.def.datalog.push(DatalogEntry {
            channel: str_tok(&tokens, 0, line.num)?,
            label: str_tok(&tokens, 1, line.num)?,
            ty,
            format: str_tok(&tokens, 3, line.num)?,
            condition,
        });
    }
    Ok(())
}

// --------------------------------------------------------------------- Menu

/// The `{ expr }` tokens of a menu/dialog line in order, each parsed.
/// Empty `{}` slots and parse failures (warned) yield `None` — the item
/// then defaults to enabled/visible.
fn conditions(tokens: &[Token], num: u32, ctx: &mut Ctx) -> Vec<Option<Expr>> {
    let srcs: Vec<&str> = tokens
        .iter()
        .filter_map(|t| match t {
            Token::Expr(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    srcs.into_iter()
        .map(|src| {
            if src.is_empty() {
                return None;
            }
            match expr::parse(src, num) {
                Ok(e) => Some(e),
                Err(e) => {
                    ctx.warn(num, format!("unparsable condition: {e}"));
                    None
                }
            }
        })
        .collect()
}

fn first_condition(tokens: &[Token], num: u32, ctx: &mut Ctx) -> Option<Expr> {
    conditions(tokens, num, ctx).into_iter().flatten().next()
}

/// `target[, "Label"][, page][, { enable }]`. Tolerates the fixture's
/// missing-comma lines (`dwell_tblMap "Dwell Map"`) by splitting the first
/// token; the optional std-editor page number is ignored.
fn menu_entry(tokens: &[Token], num: u32, ctx: &mut Ctx) -> Option<MenuEntry> {
    let first = match tokens.first() {
        Some(Token::Bare(s)) if !s.is_empty() => s.clone(),
        _ => {
            ctx.warn(num, "menu entry without a target");
            return None;
        }
    };
    let (target, inline_label) = match first.split_once('"') {
        Some((t, rest)) => (
            t.trim().to_string(),
            Some(rest.trim_end_matches('"').to_string()),
        ),
        None => (first, None),
    };
    let label = inline_label
        .or_else(|| {
            tokens.iter().find_map(|t| match t {
                Token::Str(s) => Some(s.clone()),
                _ => None,
            })
        })
        .unwrap_or_else(|| target.clone());
    Some(MenuEntry {
        target,
        label,
        enable: first_condition(&tokens[1..], num, ctx),
    })
}

pub fn menu(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        match key {
            // Single-controller INIs only use the "main" menu dialog.
            "menuDialog" => {}
            "menu" => ctx.def.menus.push(MenuDef {
                title: str_tok(&tokens, 0, line.num)?.replace('&', ""),
                items: Vec::new(),
            }),
            "subMenu" | "groupMenu" | "groupChildMenu" => {
                let Some(menu) = ctx.def.menus.last_mut() else {
                    ctx.warn(line.num, format!("`{key}` before any `menu`"));
                    continue;
                };
                if key == "groupMenu" {
                    menu.items.push(MenuItem::Group {
                        label: str_tok(&tokens, 0, line.num)?,
                        children: Vec::new(),
                    });
                    continue;
                }
                // Borrow of ctx ends here so menu_entry can warn.
                let menu_idx = ctx.def.menus.len() - 1;
                if matches!(tokens.first(), Some(Token::Bare(s)) if s == "std_separator") {
                    ctx.def.menus[menu_idx].items.push(MenuItem::Separator);
                    continue;
                }
                let Some(entry) = menu_entry(&tokens, line.num, ctx) else {
                    continue;
                };
                let items = &mut ctx.def.menus[menu_idx].items;
                if key == "groupChildMenu" {
                    match items.last_mut() {
                        Some(MenuItem::Group { children, .. }) => children.push(entry),
                        _ => ctx.warn(line.num, "`groupChildMenu` outside a `groupMenu`"),
                    }
                } else {
                    items.push(MenuItem::Entry(entry));
                }
            }
            _ => ctx.warn(line.num, format!("unknown [Menu] key `{key}`")),
        }
    }
    Ok(())
}

// -------------------------------------------------------------- UserDefined

pub fn user_defined(lines: &[Line], ctx: &mut Ctx) -> Result<(), Error> {
    for line in lines {
        let Some((key, tokens)) = kv(line, ctx) else {
            continue;
        };
        match key {
            "dialog" => {
                let name = str_tok(&tokens, 0, line.num)?;
                let title = tokens
                    .iter()
                    .find_map(|t| match t {
                        Token::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                ctx.def.dialogs.insert(
                    name.clone(),
                    DialogDef {
                        name,
                        title,
                        items: Vec::new(),
                        topic_help: None,
                        line: line.num,
                    },
                );
            }
            "field" | "slider" | "displayOnlyField" | "panel" | "topicHelp" => {
                if ctx.def.dialogs.last().is_none() {
                    ctx.warn(line.num, format!("`{key}` before any `dialog`"));
                    continue;
                }
                let item = match key {
                    "field" | "slider" => {
                        let label = str_tok(&tokens, 0, line.num).unwrap_or_default();
                        // The constant is the next bare word; sliders also
                        // carry a bare orientation token after it.
                        let constant = match tokens.get(1) {
                            Some(Token::Bare(s)) if !s.is_empty() => Some(s.clone()),
                            _ => None,
                        };
                        // field = label, name, { enable }, { visible }
                        let mut conds = conditions(&tokens[1..], line.num, ctx).into_iter();
                        let enable = conds.next().flatten();
                        let visible = conds.next().flatten();
                        Some(DialogItem::Field {
                            label,
                            constant,
                            enable,
                            visible,
                        })
                    }
                    "displayOnlyField" => {
                        let label = str_tok(&tokens, 0, line.num).unwrap_or_default();
                        let constant = match tokens.get(1) {
                            Some(Token::Bare(s)) if !s.is_empty() => Some(s.clone()),
                            _ => None,
                        };
                        let mut conds = conditions(&tokens[1..], line.num, ctx).into_iter();
                        let enable = conds.next().flatten();
                        let visible = conds.next().flatten();
                        Some(DialogItem::DisplayOnly {
                            label,
                            constant,
                            enable,
                            visible,
                        })
                    }
                    "panel" => match tokens.first() {
                        Some(Token::Bare(s)) if !s.is_empty() => Some(DialogItem::Panel {
                            target: s.clone(),
                            enable: first_condition(&tokens[1..], line.num, ctx),
                        }),
                        _ => {
                            ctx.warn(line.num, "panel without a target");
                            None
                        }
                    },
                    _ => {
                        let help = str_tok(&tokens, 0, line.num).ok();
                        let (_, dialog) = ctx.def.dialogs.last_mut().unwrap();
                        dialog.topic_help = help;
                        None
                    }
                };
                if let Some(item) = item {
                    let (_, dialog) = ctx.def.dialogs.last_mut().unwrap();
                    dialog.items.push(item);
                }
            }
            // Visual/interactive elements a settings form can't represent.
            "commandButton" | "gauge" | "liveGraph" | "graphLine" | "indicator"
            | "indicatorPanel" | "text" | "settingSelector" | "settingOption" | "radio"
            | "help" | "webHelp" => {}
            _ => ctx.warn(line.num, format!("unknown [UserDefined] key `{key}`")),
        }
    }
    Ok(())
}
