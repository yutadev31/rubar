pub(crate) fn parse_rgba(value: &str) -> Result<[u8; 4], String> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if !matches!(hex.len(), 6 | 8) || !hex.is_ascii() {
        return Err(format!(
            "invalid color `{value}`; expected #RRGGBB or #RRGGBBAA"
        ));
    }
    let component = |start| {
        u8::from_str_radix(&hex[start..start + 2], 16)
            .map_err(|_| format!("invalid color `{value}`; expected #RRGGBB or #RRGGBBAA"))
    };
    Ok([
        component(0)?,
        component(2)?,
        component(4)?,
        if hex.len() == 8 { component(6)? } else { 255 },
    ])
}
