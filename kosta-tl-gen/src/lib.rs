//! Парсер TL-схемы KoSta и генератор Rust-кода.
//!
//! Поддерживается подмножество TL, достаточное для проекта:
//!
//! * секции `---types---` / `---functions---`;
//! * объявления `name#id? arg:type ... = ResultType;`;
//! * типы `int`, `long`, `int128`, `int256`, `double`, `string`, `bytes`,
//!   `Bool`, `Object`, `vector<T>` и bare-типы схемы;
//! * `id` необязателен: без него берётся CRC32 от нормализованного
//!   объявления (свои прикладные конструкторы).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Тип TL-поля.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlType {
    Int,
    Long,
    Int128,
    Int256,
    Double,
    String,
    Bytes,
    Bool,
    Object,
    Vector(Box<TlType>),
    Bare(String),
}

impl TlType {
    /// Каноническая запись типа (для id и сообщений об ошибках).
    pub fn canonical(&self) -> String {
        match self {
            TlType::Int => "int".into(),
            TlType::Long => "long".into(),
            TlType::Int128 => "int128".into(),
            TlType::Int256 => "int256".into(),
            TlType::Double => "double".into(),
            TlType::String => "string".into(),
            TlType::Bytes => "bytes".into(),
            TlType::Bool => "Bool".into(),
            TlType::Object => "Object".into(),
            TlType::Vector(inner) => format!("vector<{}>", inner.canonical()),
            TlType::Bare(name) => name.clone(),
        }
    }

    fn rust(&self) -> String {
        match self {
            TlType::Int => "i32".into(),
            TlType::Long => "i64".into(),
            TlType::Int128 => "Int128".into(),
            TlType::Int256 => "Int256".into(),
            TlType::Double => "f64".into(),
            TlType::String => "String".into(),
            TlType::Bytes => "Bytes".into(),
            TlType::Bool => "bool".into(),
            TlType::Object => "Box<TlObject>".into(),
            TlType::Vector(inner) => format!("Vec<{}>", inner.rust()),
            TlType::Bare(name) => camel_case(name),
        }
    }

    fn read_expr(&self, reader: &str) -> String {
        match self {
            TlType::Vector(inner) => {
                format!("Vec::<{}>::read_bytes({})?", inner.rust(), reader)
            }
            TlType::Object => format!("Box::<TlObject>::read_bytes({})?", reader),
            other => format!("<{}>::read_bytes({})?", other.rust(), reader),
        }
    }

    fn contains_double(&self) -> bool {
        match self {
            TlType::Double => true,
            TlType::Vector(inner) => inner.contains_double(),
            _ => false,
        }
    }
}

/// Поле конструктора.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Имя поля (snake_case).
    pub name: String,
    /// Тип поля.
    pub ty: TlType,
}

/// Конструктор или метод TL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constructor {
    /// Имя конструктора/метода.
    pub name: String,
    /// TL-идентификатор.
    pub id: u32,
    /// Поля.
    pub fields: Vec<Field>,
    /// Имя типа результата.
    pub result: String,
    /// Метод (`---functions---`), а не тип.
    pub is_function: bool,
}

/// Разобранная схема.
#[derive(Debug, Clone, Default)]
pub struct Schema {
    /// Все конструкторы и методы в порядке объявления.
    pub constructors: Vec<Constructor>,
    /// Типы в порядке первого появления: (имя типа, индексы конструкторов).
    pub types: Vec<(String, Vec<usize>)>,
}

impl Schema {
    fn constructor(&self, index: usize) -> &Constructor {
        &self.constructors[index]
    }
}

/// Стандартный CRC32 (IEEE, отражённый полином 0xEDB88320).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// `server_DH_params_ok` → `ServerDhParamsOk`, `P_Q_inner_data` → `PqInnerData`.
pub fn camel_case(raw: &str) -> String {
    let words = split_words(raw);
    let mut merged: Vec<String> = Vec::new();
    let mut index = 0;
    while index < words.len() {
        // Одиночные буквы (`p_q_...`) склеиваем между собой: P + Q -> Pq.
        if words[index].len() == 1 && index + 1 < words.len() && words[index + 1].len() == 1 {
            merged.push(format!("{}{}", words[index], words[index + 1]));
            index += 2;
        } else {
            merged.push(words[index].clone());
            index += 1;
        }
    }
    merged
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    let mut result = String::new();
                    result.push(first.to_ascii_uppercase());
                    result.push_str(&chars.as_str().to_ascii_lowercase());
                    result
                }
                None => String::new(),
            }
        })
        .collect()
}

/// `server_DH_params_ok` → `SERVER_DH_PARAMS_OK`.
pub fn screaming_snake(raw: &str) -> String {
    split_words(raw)
        .into_iter()
        .map(|word| word.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join("_")
}

fn split_words(raw: &str) -> Vec<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, &ch) in chars.iter().enumerate() {
        if ch == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        let previous_is_lower = index > 0 && chars[index - 1].is_lowercase();
        if ch.is_uppercase() && previous_is_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn const_name(constructor: &str) -> String {
    format!("{}_ID", screaming_snake(constructor))
}

fn is_ident(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Разбирает TL-схему.
pub fn parse_schema(source: &str) -> Result<Schema, String> {
    let mut schema = Schema::default();
    let mut section: Option<bool> = None; // Some(is_function)
    let mut buffer = String::new();

    for (line_index, raw_line) in source.lines().enumerate() {
        let line_number = line_index + 1;
        let line = match raw_line.find("//") {
            Some(position) => &raw_line[..position],
            None => raw_line,
        }
        .trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("---") && line.ends_with("---") {
            if !buffer.trim().is_empty() {
                return Err(format!(
                    "line {line_number}: section marker inside a declaration"
                ));
            }
            section = Some(match line {
                "---types---" => false,
                "---functions---" => true,
                other => return Err(format!("line {line_number}: unknown section `{other}`")),
            });
            continue;
        }
        let is_function =
            section.ok_or_else(|| format!("line {line_number}: declaration before any section"))?;

        if !buffer.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(line);
        if !line.ends_with(';') {
            continue;
        }

        let declaration = std::mem::take(&mut buffer);
        let constructor = parse_declaration(&declaration, is_function)
            .map_err(|e| format!("line {line_number}: {e}"))?;
        schema.constructors.push(constructor);
    }

    if !buffer.trim().is_empty() {
        return Err(format!(
            "unterminated declaration at end of schema: `{}`",
            buffer.trim()
        ));
    }
    if schema.constructors.is_empty() {
        return Err("schema contains no constructors".into());
    }

    validate(&mut schema)?;
    Ok(schema)
}

fn parse_declaration(declaration: &str, is_function: bool) -> Result<Constructor, String> {
    let declaration = declaration.trim();
    let declaration = declaration
        .strip_suffix(';')
        .ok_or_else(|| format!("declaration must end with `;`: `{declaration}`"))?
        .trim();
    let (left, result) = declaration
        .split_once('=')
        .ok_or_else(|| format!("missing `=` in `{declaration}`"))?;
    let result = result.trim();
    if !is_ident(result) {
        return Err(format!("invalid result type `{result}`"));
    }

    let mut tokens = left.split_whitespace();
    let name_token = tokens
        .next()
        .ok_or_else(|| "empty declaration".to_string())?;
    let (name, explicit_id) = match name_token.split_once('#') {
        Some((name, id)) => {
            if id.is_empty() || id.len() > 8 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("invalid constructor id `{id}`"));
            }
            let id = u32::from_str_radix(id, 16)
                .map_err(|_| format!("invalid constructor id `{id}`"))?;
            (name, Some(id))
        }
        None => (name_token, None),
    };
    if !is_ident(name) {
        return Err(format!("invalid constructor name `{name}`"));
    }

    let mut fields = Vec::new();
    let mut seen_fields = HashSet::new();
    for token in tokens {
        let (field_name, type_source) = token
            .split_once(':')
            .ok_or_else(|| format!("argument `{token}` must have the form `name:type`"))?;
        if !is_ident(field_name) {
            return Err(format!("invalid field name `{field_name}`"));
        }
        if !seen_fields.insert(field_name.to_string()) {
            return Err(format!("duplicate field `{field_name}` in `{name}`"));
        }
        let ty = parse_type(type_source)?;
        fields.push(Field {
            name: field_name.to_string(),
            ty,
        });
    }

    let normalized = normalized_declaration(name, &fields, result);
    let id = explicit_id.unwrap_or_else(|| crc32(normalized.as_bytes()));

    Ok(Constructor {
        name: name.to_string(),
        id,
        fields,
        result: result.to_string(),
        is_function,
    })
}

fn normalized_declaration(name: &str, fields: &[Field], result: &str) -> String {
    let args = fields
        .iter()
        .map(|field| format!("{}:{}", field.name, field.ty.canonical()))
        .collect::<Vec<_>>()
        .join(" ");
    if args.is_empty() {
        format!("{name} = {result}")
    } else {
        format!("{name} {args} = {result}")
    }
}

fn parse_type(source: &str) -> Result<TlType, String> {
    let source = source.trim();
    if let Some(inner) = source
        .strip_prefix("vector<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return Ok(TlType::Vector(Box::new(parse_type(inner)?)));
    }
    Ok(match source {
        "int" => TlType::Int,
        "long" => TlType::Long,
        "int128" => TlType::Int128,
        "int256" => TlType::Int256,
        "double" => TlType::Double,
        "string" => TlType::String,
        "bytes" => TlType::Bytes,
        "Bool" | "bool" => TlType::Bool,
        "Object" | "object" => TlType::Object,
        other => {
            if !is_ident(other) {
                return Err(format!("invalid type `{other}`"));
            }
            TlType::Bare(other.to_string())
        }
    })
}

fn validate(schema: &mut Schema) -> Result<(), String> {
    let mut ids = HashMap::new();
    let mut names = HashMap::new();

    for constructor in &schema.constructors {
        if let Some(previous) = ids.insert(constructor.id, constructor.name.clone()) {
            return Err(format!(
                "constructor id 0x{:08x} is used by both `{previous}` and `{}`",
                constructor.id, constructor.name
            ));
        }
        if let Some(previous) = names.insert(constructor.name.clone(), constructor.id) {
            return Err(format!(
                "constructor name `{}` is declared twice (ids 0x{previous:08x} and 0x{:08x})",
                constructor.name, constructor.id
            ));
        }
    }

    let mut type_names: HashSet<String> = HashSet::new();
    for constructor in &schema.constructors {
        if !constructor.is_function {
            type_names.insert(constructor.result.clone());
        }
    }

    for constructor in &schema.constructors {
        if !constructor.is_function && constructor.result == "Bool" {
            return Err(format!(
                "type constructor `{}` cannot return the primitive Bool",
                constructor.name
            ));
        }
        for field in &constructor.fields {
            check_type_reference(
                &field.ty,
                &type_names,
                &format!("{}.{}", constructor.name, field.name),
            )?;
        }
    }

    for constructor in &schema.constructors {
        if constructor.is_function
            && !is_primitive_result(&constructor.result)
            && !type_names.contains(&constructor.result)
        {
            return Err(format!(
                "function `{}` returns unknown type `{}`",
                constructor.name, constructor.result
            ));
        }
    }

    let mut types: Vec<(String, Vec<usize>)> = Vec::new();
    let mut index_by_name: HashMap<String, usize> = HashMap::new();
    for (index, constructor) in schema.constructors.iter().enumerate() {
        if constructor.is_function {
            continue;
        }
        match index_by_name.get(&constructor.result) {
            Some(&type_index) => types[type_index].1.push(index),
            None => {
                index_by_name.insert(constructor.result.clone(), types.len());
                types.push((constructor.result.clone(), vec![index]));
            }
        }
    }
    schema.types = types;
    Ok(())
}

fn is_primitive_result(result: &str) -> bool {
    matches!(result, "Bool" | "Object")
}

fn check_type_reference(
    ty: &TlType,
    type_names: &HashSet<String>,
    context: &str,
) -> Result<(), String> {
    match ty {
        TlType::Bare(name) => {
            if type_names.contains(name) {
                Ok(())
            } else {
                Err(format!("{context}: unknown type `{name}`"))
            }
        }
        TlType::Vector(inner) => check_type_reference(inner, type_names, context),
        _ => Ok(()),
    }
}

/// Генерирует Rust-код по разобранной схеме.
pub fn generate(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str("// @generated by kosta-tl-gen from schema/kosta.tl. Do not edit manually.\n\n");
    out.push_str("use crate::tl::*;\n\n");

    for constructor in &schema.constructors {
        let _ = writeln!(
            out,
            "/// Constructor id of `{}`.\npub const {}: u32 = 0x{:08X};",
            constructor.name,
            const_name(&constructor.name),
            constructor.id
        );
    }
    out.push('\n');

    for (type_name, indices) in &schema.types {
        let constructors: Vec<&Constructor> =
            indices.iter().map(|&i| schema.constructor(i)).collect();
        if constructors.len() == 1 {
            emit_struct(&mut out, constructors[0], &camel_case(type_name));
        } else {
            emit_enum(&mut out, type_name, &constructors);
        }
        out.push('\n');
    }

    for constructor in schema.constructors.iter().filter(|c| c.is_function) {
        emit_struct(&mut out, constructor, &camel_case(&constructor.name));
        out.push('\n');
    }

    emit_tl_object(&mut out, schema);
    out
}

fn derive_line(fields: &[Field]) -> &'static str {
    if fields.iter().any(|field| field.ty.contains_double()) {
        "#[derive(Debug, Clone, PartialEq)]"
    } else {
        "#[derive(Debug, Clone, PartialEq, Eq)]"
    }
}

fn emit_struct(out: &mut String, constructor: &Constructor, struct_name: &str) {
    let id_const = const_name(&constructor.name);
    let _ = writeln!(out, "{}", derive_line(&constructor.fields));
    if constructor.fields.is_empty() {
        let _ = writeln!(out, "pub struct {struct_name};");
        let _ = writeln!(out, "impl {struct_name} {{");
        let _ = writeln!(out, "    /// TL constructor id.");
        let _ = writeln!(out, "    pub const ID: u32 = {id_const};");
        let _ = writeln!(
            out,
            "    pub(crate) fn read_fields(reader: &mut Reader<'_>) -> Result<Self, TlError> {{"
        );
        let _ = writeln!(out, "        let _ = reader;");
        let _ = writeln!(out, "        Ok(Self)");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
    } else {
        let _ = writeln!(out, "pub struct {struct_name} {{");
        for field in &constructor.fields {
            let _ = writeln!(out, "    pub {}: {},", field.name, field.ty.rust());
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out, "impl {struct_name} {{");
        let _ = writeln!(out, "    /// TL constructor id.");
        let _ = writeln!(out, "    pub const ID: u32 = {id_const};");
        let _ = writeln!(
            out,
            "    pub(crate) fn read_fields(reader: &mut Reader<'_>) -> Result<Self, TlError> {{"
        );
        let _ = writeln!(out, "        Ok(Self {{");
        for field in &constructor.fields {
            let _ = writeln!(
                out,
                "            {}: {},",
                field.name,
                field.ty.read_expr("reader")
            );
        }
        let _ = writeln!(out, "        }})");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
    }

    let _ = writeln!(out, "impl TlWrite for {struct_name} {{");
    let _ = writeln!(
        out,
        "    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {{"
    );
    let _ = writeln!(out, "        write_i32(out, {id_const} as i32);");
    for field in &constructor.fields {
        let _ = writeln!(out, "        self.{}.write_bytes(out)?;", field.name);
    }
    let _ = writeln!(out, "        Ok(())");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "impl TlRead for {struct_name} {{");
    let _ = writeln!(
        out,
        "    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {{"
    );
    let _ = writeln!(out, "        let id = reader.read_u32()?;");
    let _ = writeln!(
        out,
        "        if id != {id_const} {{ return Err(TlError::UnknownConstructor(id)); }}"
    );
    let _ = writeln!(out, "        Self::read_fields(reader)");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");
}

fn emit_enum(out: &mut String, type_name: &str, constructors: &[&Constructor]) {
    let enum_name = camel_case(type_name);
    let _ = writeln!(out, "#[allow(clippy::enum_variant_names)]");
    out.push_str("#[derive(Debug, Clone, PartialEq, Eq)]\n");
    let _ = writeln!(out, "pub enum {enum_name} {{");
    for constructor in constructors {
        let variant = camel_case(&constructor.name);
        if constructor.fields.is_empty() {
            let _ = writeln!(out, "    {variant},");
        } else {
            let _ = writeln!(out, "    {variant} {{");
            for field in &constructor.fields {
                let _ = writeln!(out, "        {}: {},", field.name, field.ty.rust());
            }
            let _ = writeln!(out, "    }},");
        }
    }
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "impl {enum_name} {{");
    let _ = writeln!(out, "    /// TL constructor id of the current variant.");
    let _ = writeln!(out, "    pub fn constructor_id(&self) -> u32 {{");
    let _ = writeln!(out, "        match self {{");
    for constructor in constructors {
        let variant = camel_case(&constructor.name);
        let id_const = const_name(&constructor.name);
        if constructor.fields.is_empty() {
            let _ = writeln!(out, "            Self::{variant} => {id_const},");
        } else {
            let _ = writeln!(out, "            Self::{variant} {{ .. }} => {id_const},");
        }
    }
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");

    let _ = writeln!(
        out,
        "    pub(crate) fn read_fields(id: u32, reader: &mut Reader<'_>) -> Result<Self, TlError> {{"
    );
    if !constructors
        .iter()
        .any(|constructor| !constructor.fields.is_empty())
    {
        let _ = writeln!(out, "        let _ = reader;");
    }
    let _ = writeln!(out, "        match id {{");
    for constructor in constructors {
        let variant = camel_case(&constructor.name);
        let id_const = const_name(&constructor.name);
        if constructor.fields.is_empty() {
            let _ = writeln!(out, "            {id_const} => Ok(Self::{variant}),");
        } else {
            let _ = writeln!(out, "            {id_const} => Ok(Self::{variant} {{");
            for field in &constructor.fields {
                let _ = writeln!(
                    out,
                    "                {}: {},",
                    field.name,
                    field.ty.read_expr("reader")
                );
            }
            let _ = writeln!(out, "            }}),");
        }
    }
    let _ = writeln!(
        out,
        "            _ => Err(TlError::UnknownConstructor(id)),"
    );
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "impl TlWrite for {enum_name} {{");
    let _ = writeln!(
        out,
        "    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {{"
    );
    let _ = writeln!(out, "        match self {{");
    for constructor in constructors {
        let variant = camel_case(&constructor.name);
        let id_const = const_name(&constructor.name);
        if constructor.fields.is_empty() {
            let _ = writeln!(out, "            Self::{variant} => {{");
            let _ = writeln!(out, "                write_i32(out, {id_const} as i32);");
            let _ = writeln!(out, "            }}");
        } else {
            let names: Vec<&str> = constructor.fields.iter().map(|f| f.name.as_str()).collect();
            let _ = writeln!(
                out,
                "            Self::{variant} {{ {} }} => {{",
                names.join(", ")
            );
            let _ = writeln!(out, "                write_i32(out, {id_const} as i32);");
            for field in &constructor.fields {
                let _ = writeln!(out, "                {}.write_bytes(out)?;", field.name);
            }
            let _ = writeln!(out, "            }}");
        }
    }
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        Ok(())");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "impl TlRead for {enum_name} {{");
    let _ = writeln!(
        out,
        "    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {{"
    );
    let _ = writeln!(out, "        let id = reader.read_u32()?;");
    let _ = writeln!(out, "        Self::read_fields(id, reader)");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");
}

fn emit_tl_object(out: &mut String, schema: &Schema) {
    out.push_str("#[allow(clippy::large_enum_variant)]\n#[derive(Debug, Clone, PartialEq, Eq)]\n");
    out.push_str("pub enum TlObject {\n");
    for (type_name, indices) in &schema.types {
        let variant = camel_case(type_name);
        let _ = writeln!(out, "    {variant}({variant}),");
        let _ = indices;
    }
    for constructor in schema.constructors.iter().filter(|c| c.is_function) {
        let variant = camel_case(&constructor.name);
        let _ = writeln!(out, "    {variant}({variant}),");
    }
    out.push_str("}\n\n");

    let _ = writeln!(out, "impl TlObject {{");
    let _ = writeln!(out, "    /// TL constructor id of the current object.");
    let _ = writeln!(out, "    pub fn constructor_id(&self) -> u32 {{");
    let _ = writeln!(out, "        match self {{");
    for (type_name, indices) in &schema.types {
        let variant = camel_case(type_name);
        if indices.len() == 1 {
            let id_const = const_name(&schema.constructor(indices[0]).name);
            let _ = writeln!(out, "            TlObject::{variant}(_) => {id_const},");
        } else {
            let _ = writeln!(
                out,
                "            TlObject::{variant}(value) => value.constructor_id(),"
            );
        }
    }
    for constructor in schema.constructors.iter().filter(|c| c.is_function) {
        let variant = camel_case(&constructor.name);
        let id_const = const_name(&constructor.name);
        let _ = writeln!(out, "            TlObject::{variant}(_) => {id_const},");
    }
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(
        out,
        "    /// Читает boxed TL-объект (с идентификатором конструктора)."
    );
    let _ = writeln!(
        out,
        "    pub fn read_boxed(reader: &mut Reader<'_>) -> Result<Self, TlError> {{"
    );
    let _ = writeln!(out, "        let id = reader.read_u32()?;");
    let _ = writeln!(out, "        Self::read_boxed_with_id(id, reader)");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(
        out,
        "    /// Читает boxed TL-объект, когда id уже прочитан."
    );
    let _ = writeln!(
        out,
        "    pub fn read_boxed_with_id(id: u32, reader: &mut Reader<'_>) -> Result<Self, TlError> {{"
    );
    let _ = writeln!(out, "        match id {{");
    for (type_name, indices) in &schema.types {
        let variant = camel_case(type_name);
        if indices.len() == 1 {
            let id_const = const_name(&schema.constructor(indices[0]).name);
            let _ = writeln!(
                out,
                "            {id_const} => Ok(TlObject::{variant}({variant}::read_fields(reader)?)),"
            );
        } else {
            let patterns = indices
                .iter()
                .map(|&index| const_name(&schema.constructor(index).name))
                .collect::<Vec<_>>()
                .join(" | ");
            let _ = writeln!(
                out,
                "            {patterns} => Ok(TlObject::{variant}({variant}::read_fields(id, reader)?)),"
            );
        }
    }
    for constructor in schema.constructors.iter().filter(|c| c.is_function) {
        let variant = camel_case(&constructor.name);
        let id_const = const_name(&constructor.name);
        let _ = writeln!(
            out,
            "            {id_const} => Ok(TlObject::{variant}({variant}::read_fields(reader)?)),"
        );
    }
    let _ = writeln!(
        out,
        "            _ => Err(TlError::UnknownConstructor(id)),"
    );
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");

    out.push_str(
        "impl TlWrite for TlObject {\n    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {\n        match self {\n",
    );
    for (type_name, _) in &schema.types {
        let variant = camel_case(type_name);
        let _ = writeln!(
            out,
            "            TlObject::{variant}(value) => value.write_bytes(out),"
        );
    }
    for constructor in schema.constructors.iter().filter(|c| c.is_function) {
        let variant = camel_case(&constructor.name);
        let _ = writeln!(
            out,
            "            TlObject::{variant}(value) => value.write_bytes(out),"
        );
    }
    out.push_str("        }\n    }\n}\n\nimpl TlRead for TlObject {\n    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {\n        Self::read_boxed(reader)\n    }\n}\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
---types---
resPQ#05162463 nonce:int128 server_nonce:int128 pq:bytes fingerprints:vector<long> = ResPQ;
message#5bb8e511 msg_id:long seqno:int bytes:int body:bytes = Message;
msg_container#73f1f8dc messages:vector<Message> = MsgContainer;
chatMessage id:long text:string is_outgoing:Bool = ChatMessage;
chatMessageEmpty = ChatMessage;

---functions---
req_pq#60469778 nonce:int128 = ResPQ;
sendMessage chat_id:long text:string = ChatMessage;
getDialogs = ChatMessage;
"#;

    #[test]
    fn crc32_matches_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn camel_and_screaming_names() {
        assert_eq!(camel_case("resPQ"), "ResPq");
        assert_eq!(camel_case("server_DH_params_ok"), "ServerDhParamsOk");
        assert_eq!(camel_case("P_Q_inner_data"), "PqInnerData");
        assert_eq!(camel_case("sendMessage"), "SendMessage");
        assert_eq!(
            screaming_snake("server_DH_params_ok"),
            "SERVER_DH_PARAMS_OK"
        );
        assert_eq!(screaming_snake("sendMessage"), "SEND_MESSAGE");
    }

    #[test]
    fn parser_reads_explicit_and_generated_ids() {
        let schema = parse_schema(SAMPLE).unwrap();
        assert_eq!(schema.constructors.len(), 8);
        assert_eq!(schema.constructors[0].id, 0x0516_2463);

        let generated = schema
            .constructors
            .iter()
            .find(|c| c.name == "sendMessage")
            .unwrap();
        assert_eq!(
            generated.id,
            crc32(b"sendMessage chat_id:long text:string = ChatMessage")
        );
        assert!(schema.types.iter().any(|(name, _)| name == "ChatMessage"));
    }

    #[test]
    fn parser_rejects_duplicate_ids() {
        let source = "---types---\na#00000001 = A;\nb#00000001 = B;\n";
        assert!(parse_schema(source).unwrap_err().contains("used by both"));
    }

    #[test]
    fn parser_rejects_unknown_types() {
        let source = "---types---\na x:Unknown = A;\n";
        assert!(parse_schema(source).unwrap_err().contains("unknown type"));
    }

    #[test]
    fn parser_rejects_bad_syntax() {
        assert!(parse_schema("---types---\na#zzzz = A;").is_err());
        assert!(parse_schema("a#00000001 = A;").is_err());
        assert!(parse_schema("---types---\na x = A;").is_err());
        assert!(parse_schema("---types---\na x:int = A").is_err());
    }

    #[test]
    fn generator_emits_expected_items() {
        let schema = parse_schema(SAMPLE).unwrap();
        let code = generate(&schema);
        for needle in [
            "pub const RES_PQ_ID: u32 = 0x05162463;",
            "pub struct ResPq {",
            "pub enum ChatMessage {",
            "pub struct ReqPq",
            "pub enum TlObject {",
            "impl TlWrite for TlObject",
            "pub fn read_boxed(",
            "pub const SEND_MESSAGE_ID",
        ] {
            assert!(
                code.contains(needle),
                "missing `{needle}` in generated code"
            );
        }
    }

    #[test]
    fn real_schema_parses_and_has_canonical_ids() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../schema/kosta.tl");
        let source = std::fs::read_to_string(path).expect("read schema/kosta.tl");
        let schema = parse_schema(&source).expect("schema must parse");

        let canonical = [
            ("resPQ", 0x0516_2463u32),
            ("req_pq", 0x6046_9778),
            ("req_DH_params", 0xd712_e4be),
            ("server_DH_params_ok", 0xd0e6_0441),
            ("server_DH_inner_data", 0xb589_0dba),
            ("set_client_DH_params", 0xf504_5f1f),
            ("dh_gen_ok", 0x3bcb_f734),
            ("msg_container", 0x73f1_f8dc),
            ("message", 0x5bb8_e511),
            ("msgs_ack", 0x62d6_b459),
            ("rpc_result", 0xf35c_6d01),
            ("rpc_error", 0x2144_ca19),
            ("new_session_created", 0x9ec2_0908),
        ];
        for (name, expected) in canonical {
            let constructor = schema
                .constructors
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("missing constructor `{name}`"));
            assert_eq!(constructor.id, expected, "wrong id for `{name}`");
        }
    }
}
