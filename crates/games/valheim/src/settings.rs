use gsm_domain::settings::{Field, Kind};
pub fn schema() -> Vec<Field> {
    let mut fields = gsm_domain::settings::paths(true, true);
    fields.extend([
        Field::new("name", "manager", "server", "name", Kind::Text, ""),
        Field::new(
            "port",
            "manager",
            "server",
            "port",
            Kind::Number(1024, 65534),
            "2456",
        ),
        Field::new(
            "password",
            "manager",
            "server",
            "password",
            Kind::Secret,
            "",
        ),
        Field::new(
            "save_interval",
            "manager",
            "server",
            "save_interval",
            Kind::Number(1, 86400),
            "1800",
        ),
        Field::new(
            "backups",
            "manager",
            "server",
            "backups",
            Kind::Number(0, 1000),
            "4",
        ),
        Field::new(
            "public",
            "manager",
            "server",
            "public",
            Kind::Number(0, 1),
            "1",
        ),
        Field::new(
            "crossplay",
            "manager",
            "server",
            "crossplay",
            Kind::Bool,
            "false",
        ),
    ]);
    fields
}
pub fn creation_schema() -> Vec<Field> {
    let mut fields = schema();
    fields.push(Field::new(
        "new_world",
        "manager",
        "server",
        "world",
        Kind::Text,
        "NewWorld",
    ));
    fields
}
pub fn create(values: &[(String, String)]) -> Result<gsm_domain::settings::Documents, String> {
    let docs = gsm_domain::settings::new_documents(&creation_schema(), Default::default(), values)?;
    let doc =
        crate::config::ConfigDocument::parse(docs["manager"].clone()).map_err(|e| e.to_string())?;
    crate::launch::build_launch_plan(doc.settings()).map_err(|e| e.to_string())?;
    Ok(docs)
}
