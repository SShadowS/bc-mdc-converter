// MDC domain types decoded from the FlatBuffers schema (algorithm spec, .mdc schema spec).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectType {
    TableData = 0,
    Table = 1,
    Form = 2,
    Report = 3,
    Dataport = 4,
    CodeUnit = 5,
    XmlPort = 6,
    MenuSuite = 7,
    Page = 8,
    Query = 9,
    System = 10,
    FieldNumber = 11,
    LimitedUsageTableData = 12,
    TablePage = 13,
    PageExtension = 14,
    TableExtension = 15,
    Enum = 16,
    EnumExtension = 17,
    ReportExtension = 22,
    Interface = 28,
}
impl ObjectType {
    pub fn from_i8(v: i8) -> ObjectType {
        use ObjectType::*;
        match v {
            1 => Table,
            2 => Form,
            3 => Report,
            4 => Dataport,
            5 => CodeUnit,
            6 => XmlPort,
            7 => MenuSuite,
            8 => Page,
            9 => Query,
            10 => System,
            11 => FieldNumber,
            12 => LimitedUsageTableData,
            13 => TablePage,
            14 => PageExtension,
            15 => TableExtension,
            16 => Enum,
            17 => EnumExtension,
            22 => ReportExtension,
            28 => Interface,
            _ => TableData,
        }
    }
    /// scriptId / applicationDefinition.objectType string (algorithm spec §7.4/§7.5).
    pub fn name(self) -> &'static str {
        use ObjectType::*;
        match self {
            TableData => "TableData",
            Table => "Table",
            Form => "Form",
            Report => "Report",
            Dataport => "Dataport",
            CodeUnit => "CodeUnit",
            XmlPort => "XmlPort",
            MenuSuite => "MenuSuite",
            Page => "Page",
            Query => "Query",
            System => "System",
            FieldNumber => "FieldNumber",
            LimitedUsageTableData => "LimitedUsageTableData",
            TablePage => "TablePage",
            PageExtension => "PageExtension",
            TableExtension => "TableExtension",
            Enum => "Enum",
            EnumExtension => "EnumExtension",
            ReportExtension => "ReportExtension",
            Interface => "Interface",
        }
    }
    /// Firefox object-type DISPLAY name (Firefox emitter spec §2.4/§3.1/§8). Identical to `name()` for every
    /// variant EXCEPT `CodeUnit`, which Firefox spells `"Codeunit"` (lowercase u) — the
    /// `funcTable.name` / `BcCall` marker convention throughout Firefox emitter spec. Deliberately distinct
    /// from `name()` (the V8/scriptId `"CodeUnit"`) and `symbol_kind()` (which lossily collapses
    /// `System`→`Class` and several types→`Undefined`). Exhaustive on purpose: a new
    /// `ObjectType` variant should force a compile error here, not silently pick a default.
    pub fn firefox_name(self) -> &'static str {
        use ObjectType::*;
        match self {
            CodeUnit => "Codeunit",
            TableData => "TableData",
            Table => "Table",
            Form => "Form",
            Report => "Report",
            Dataport => "Dataport",
            XmlPort => "XmlPort",
            MenuSuite => "MenuSuite",
            Page => "Page",
            Query => "Query",
            System => "System",
            FieldNumber => "FieldNumber",
            LimitedUsageTableData => "LimitedUsageTableData",
            TablePage => "TablePage",
            PageExtension => "PageExtension",
            TableExtension => "TableExtension",
            Enum => "Enum",
            EnumExtension => "EnumExtension",
            ReportExtension => "ReportExtension",
            Interface => "Interface",
        }
    }
    /// URL `kind` segment = SymbolKind name (algorithm spec §7.7, `DebugHelper.MapToSymbolKind`; full
    /// table in output format spec §220-238). NOTE `Codeunit` (lower u), unlike `name()`. Exhaustive (no
    /// catch-all) on purpose: a future `ObjectType` variant should force a compile error here
    /// rather than silently falling through to a wrong default.
    pub fn symbol_kind(self) -> &'static str {
        use ObjectType::*;
        match self {
            CodeUnit => "Codeunit",
            Table => "Table",
            Page => "Page",
            Report => "Report",
            Query => "Query",
            XmlPort => "XmlPort",
            Enum => "Enum",
            PageExtension => "PageExtension",
            TableExtension => "TableExtension",
            EnumExtension => "EnumExtension",
            ReportExtension => "ReportExtension",
            Interface => "Interface",
            // `System` maps to `Class`, not `System` (output format spec §237).
            System => "Class",
            // Types with no `MapToSymbolKind` entry fall through the C# switch's default arm to
            // `Undefined` (output format spec §238).
            TableData | Form | Dataport | MenuSuite | FieldNumber | LimitedUsageTableData
            | TablePage => "Undefined",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ApplicationObjectId {
    pub number: i32,
    pub object_type: ObjectType,
}
impl ApplicationObjectId {
    pub fn to_script_id(&self) -> String {
        format!("{}_{}", self.object_type.name(), self.number)
    }
    /// GetHashCode = ObjectNumber ^ ((int)ObjectType << 25)  (algorithm spec §7.6)
    pub fn get_hash_code(&self) -> i32 {
        self.number ^ ((self.object_type as i32) << 25)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourcePosition {
    pub line: u16,
    pub column: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceSpan {
    pub from: SourcePosition,
    pub to: SourcePosition,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeclaringApplication {
    pub app_id: String,
    pub app_name: String,
    pub app_publisher: String,
    pub app_version: String,
}

#[cfg(test)]
mod tests {
    use super::ObjectType;

    #[test]
    fn firefox_name_lowercases_only_codeunit() {
        // Firefox emitter spec spells CodeUnit as "Codeunit" (lowercase u) for the Firefox display, unlike
        // the V8/scriptId "CodeUnit" (name()). Every other type keeps its real name().
        assert_eq!(ObjectType::CodeUnit.firefox_name(), "Codeunit");
        assert_eq!(ObjectType::Table.firefox_name(), "Table");
        assert_eq!(ObjectType::System.firefox_name(), "System");
        assert_eq!(ObjectType::Report.firefox_name(), "Report");
        // Contrast: name() keeps the capital-U scriptId spelling (V8 path depends on it).
        assert_eq!(ObjectType::CodeUnit.name(), "CodeUnit");
    }

    #[test]
    fn symbol_kind_maps_per_map_to_symbol_kind_table() {
        // System maps to Class, not "System" (output format spec §237).
        assert_eq!(ObjectType::System.symbol_kind(), "Class");
        // Types with no MapToSymbolKind entry fall through to Undefined (output format spec §238).
        assert_eq!(ObjectType::TableData.symbol_kind(), "Undefined");
        assert_eq!(ObjectType::Form.symbol_kind(), "Undefined");
        // Regression: the already-correct mappings must stay put.
        assert_eq!(ObjectType::CodeUnit.symbol_kind(), "Codeunit");
        assert_eq!(ObjectType::Page.symbol_kind(), "Page");
    }
}
