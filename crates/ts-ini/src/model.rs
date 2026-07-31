//! Typed definition model produced by parsing.

use indexmap::IndexMap;

use crate::Error;
use crate::expr::{Expr, SymbolSource, Value};
use crate::lex::Shape;

/// A field that is either a literal number or a `{ expr }` evaluated later
/// (e.g. `scale = {0.1 / stoich}`, gauge `hi = {rpmhigh}`).
#[derive(Debug, Clone, PartialEq)]
pub enum NumOrExpr {
    Num(f64),
    Expr(Expr),
}

impl NumOrExpr {
    pub fn eval(&self, syms: &dyn SymbolSource) -> Result<f64, Error> {
        match self {
            NumOrExpr::Num(n) => Ok(*n),
            NumOrExpr::Expr(e) => e
                .eval(syms)?
                .as_num()
                .ok_or_else(|| Error::eval("expression yielded a string, expected number")),
        }
    }

    /// The literal value, if this doesn't need evaluation.
    pub fn literal(&self) -> Option<f64> {
        match self {
            NumOrExpr::Num(n) => Some(*n),
            NumOrExpr::Expr(_) => None,
        }
    }
}

/// A field that is either a literal string or a `{ expr }` (e.g. units
/// `{ bitStringValue(algorithmUnits, algorithm) }`).
#[derive(Debug, Clone, PartialEq)]
pub enum StrOrExpr {
    Str(String),
    Expr(Expr),
}

impl StrOrExpr {
    pub fn eval(&self, syms: &dyn SymbolSource) -> Result<String, Error> {
        match self {
            StrOrExpr::Str(s) => Ok(s.clone()),
            StrOrExpr::Expr(e) => Ok(match e.eval(syms)? {
                Value::Str(s) => s,
                Value::Num(n) => n.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    U08,
    S08,
    U16,
    S16,
    U32,
    S32,
    Ascii,
}

impl DataType {
    pub fn parse(s: &str, num: u32) -> Result<Self, Error> {
        Ok(match s {
            "U08" => DataType::U08,
            "S08" => DataType::S08,
            "U16" => DataType::U16,
            "S16" => DataType::S16,
            "U32" => DataType::U32,
            "S32" => DataType::S32,
            "ASCII" => DataType::Ascii,
            other => return Err(Error::at(num, format!("unknown data type `{other}`"))),
        })
    }

    pub fn size(&self) -> u32 {
        match self {
            DataType::U08 | DataType::S08 | DataType::Ascii => 1,
            DataType::U16 | DataType::S16 => 2,
            DataType::U32 | DataType::S32 => 4,
        }
    }

    pub fn signed(&self) -> bool {
        matches!(self, DataType::S08 | DataType::S16 | DataType::S32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstantClass {
    Scalar,
    Bits,
    Array,
    /// `name = string, ASCII, len`
    String,
}

/// One entry from [Constants] (page set) or [PcVariables] (offset `None`).
#[derive(Debug, Clone)]
pub struct ConstantDef {
    pub name: String,
    pub class: ConstantClass,
    pub ty: DataType,
    /// 1-based tune page; `None` for PcVariables.
    pub page: Option<u8>,
    /// Byte offset within the page; `None` for PcVariables.
    pub offset: Option<u32>,
    /// Bit range for `bits`, array shape for `array`.
    pub shape: Option<Shape>,
    pub units: Option<StrOrExpr>,
    pub scale: NumOrExpr,
    pub translate: NumOrExpr,
    pub lo: Option<NumOrExpr>,
    pub hi: Option<NumOrExpr>,
    pub digits: Option<NumOrExpr>,
    /// Combo-box labels for `bits` ("INVALID" entries are hidden choices).
    pub labels: Vec<String>,
    /// ASCII string length for `class == String`.
    pub string_len: Option<u32>,
    pub no_msq_save: bool,
    pub line: u32,
}

impl ConstantDef {
    /// Bytes this constant occupies in its page.
    pub fn byte_size(&self) -> u32 {
        match self.class {
            ConstantClass::Scalar | ConstantClass::Bits => self.ty.size(),
            ConstantClass::Array => self.shape.map_or(0, |s| s.element_count()) * self.ty.size(),
            ConstantClass::String => self.string_len.unwrap_or(0),
        }
    }
}

/// [Constants] header: page geometry and serial command templates. Command
/// strings are kept raw (`"p%2i%2o%2c"`, `"\$tsCanId\x01"`); ecu-proto
/// interprets the `%`/`\` codes.
#[derive(Debug, Clone, Default)]
pub struct ConstantsHeader {
    pub endianness: String,
    pub n_pages: usize,
    pub page_sizes: Vec<u32>,
    pub page_identifiers: Vec<String>,
    pub page_read_command: Vec<String>,
    pub page_value_write: Vec<String>,
    pub page_chunk_write: Vec<String>,
    pub crc32_check_command: Vec<String>,
    pub burn_command: Vec<String>,
    pub table_crc_command: Option<String>,
    pub blocking_factor: Option<u32>,
    pub delay_after_port_open: Option<u32>,
    pub block_read_timeout: Option<u32>,
    pub ts_write_blocks: bool,
    pub inter_write_delay: Option<u32>,
    pub page_activation_delay: Option<u32>,
    pub message_envelope_format: Option<String>,
    /// Header keys we don't model, preserved as raw text.
    pub misc: IndexMap<String, String>,
}

/// One [OutputChannels] entry.
#[derive(Debug, Clone)]
pub enum OutputChannel {
    Scalar {
        ty: DataType,
        offset: u32,
        units: Option<StrOrExpr>,
        scale: NumOrExpr,
        translate: NumOrExpr,
    },
    Bits {
        ty: DataType,
        offset: u32,
        lo: u8,
        hi: u8,
    },
    /// `name = { expr }` computed from other channels.
    Derived { expr: Expr, units: Option<String> },
}

#[derive(Debug, Clone)]
pub struct TableDef {
    pub id: String,
    pub map_id: String,
    pub title: String,
    pub page: u8,
    /// (bins constant, live channel for the axis cursor)
    pub x_bins: (String, Option<String>),
    pub y_bins: (String, Option<String>),
    pub z_bins: String,
    pub xy_labels: Vec<String>,
    pub grid_height: Option<f64>,
    pub grid_orient: Vec<f64>,
    pub up_down_label: Vec<String>,
    pub topic_help: Option<String>,
    pub misc: Vec<(String, String)>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct CurveDef {
    pub id: String,
    pub title: String,
    pub column_label: Vec<String>,
    /// min, max, divisions
    pub x_axis: Vec<NumOrExpr>,
    pub y_axis: Vec<NumOrExpr>,
    pub x_bins: (String, Option<String>),
    pub y_bins: Vec<String>,
    pub misc: Vec<(String, String)>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct GaugeDef {
    pub name: String,
    pub channel: String,
    pub title: String,
    pub units: String,
    pub lo: NumOrExpr,
    pub hi: NumOrExpr,
    pub lo_danger: NumOrExpr,
    pub lo_warn: NumOrExpr,
    pub hi_warn: NumOrExpr,
    pub hi_danger: NumOrExpr,
    pub value_digits: u8,
    pub label_digits: u8,
    pub category: String,
}

#[derive(Debug, Clone)]
pub struct Indicator {
    pub condition: Expr,
    pub off_label: String,
    pub on_label: String,
    /// off-bg, off-fg, on-bg, on-fg
    pub colors: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FrontPage {
    /// Gauge names in position order (gauge1..gaugeN).
    pub gauges: Vec<String>,
    pub indicators: Vec<Indicator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatalogType {
    Int,
    Float,
}

#[derive(Debug, Clone)]
pub struct DatalogEntry {
    pub channel: String,
    pub label: String,
    pub ty: DatalogType,
    pub format: String,
    /// Only log when this evaluates truthy (e.g. `{ vvtEnabled > 0 }`).
    pub condition: Option<Expr>,
}

/// One clickable [Menu] entry. `target` names a dialog, table, curve, or a
/// TunerStudio built-in editor (`std_*`); classification happens at lookup.
#[derive(Debug, Clone)]
pub struct MenuEntry {
    pub target: String,
    pub label: String,
    /// Grey the entry out unless this evaluates truthy.
    pub enable: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum MenuItem {
    Entry(MenuEntry),
    Separator,
    /// `groupMenu` with its `groupChildMenu` children.
    Group {
        label: String,
        children: Vec<MenuEntry>,
    },
}

#[derive(Debug, Clone)]
pub struct MenuDef {
    /// Menu-bar title with the `&` mnemonic marker stripped.
    pub title: String,
    pub items: Vec<MenuItem>,
}

/// [UserDefined] dialog elements we can render as a settings form. Purely
/// visual/interactive elements (gauges, live graphs, command buttons, ...)
/// are skipped at parse time.
#[derive(Debug, Clone)]
pub enum DialogItem {
    /// `field`/`slider`. `constant: None` is a header or spacer row.
    Field {
        label: String,
        constant: Option<String>,
        /// Grey out unless truthy.
        enable: Option<Expr>,
        /// Hide entirely unless truthy.
        visible: Option<Expr>,
    },
    /// A read-only constant value, optionally greyed out or hidden.
    DisplayOnly {
        label: String,
        constant: Option<String>,
        enable: Option<Expr>,
        visible: Option<Expr>,
    },
    /// Embedded sub-dialog (or curve/table editor) by name.
    Panel {
        target: String,
        enable: Option<Expr>,
    },
}

#[derive(Debug, Clone)]
pub struct DialogDef {
    pub name: String,
    pub title: String,
    pub items: Vec<DialogItem>,
    pub topic_help: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct SettingGroup {
    pub keyword: String,
    pub display_name: String,
    /// (option symbol, label); the symbol "DEFAULT" means no symbol set.
    pub options: Vec<(String, String)>,
}

/// The parsed ECU definition.
#[derive(Debug, Clone, Default)]
pub struct IniDef {
    pub mt_version: Option<f64>,
    pub query_command: String,
    pub signature: String,
    pub version_info: String,
    pub setting_groups: Vec<SettingGroup>,
    pub pc_variables: IndexMap<String, ConstantDef>,
    pub header: ConstantsHeader,
    /// All page constants in file order (each knows its page/offset).
    pub constants: IndexMap<String, ConstantDef>,
    pub requires_power_cycle: Vec<String>,
    /// (constant name, raw default text)
    pub default_values: Vec<(String, String)>,
    /// Constants the controller may change on its own (TS re-reads these).
    pub controller_priority: Vec<String>,
    pub och_get_command: String,
    pub och_block_size: u32,
    pub output_channels: IndexMap<String, OutputChannel>,
    pub tables: IndexMap<String, TableDef>,
    pub curves: IndexMap<String, CurveDef>,
    pub gauges: IndexMap<String, GaugeDef>,
    pub front_page: FrontPage,
    pub datalog: Vec<DatalogEntry>,
    pub menus: Vec<MenuDef>,
    pub dialogs: IndexMap<String, DialogDef>,
    /// Non-fatal oddities encountered while parsing.
    pub warnings: Vec<String>,
}

impl IniDef {
    /// Constants on one page, in definition order.
    pub fn page_constants(&self, page: u8) -> impl Iterator<Item = &ConstantDef> {
        self.constants
            .values()
            .filter(move |c| c.page == Some(page))
    }
}
