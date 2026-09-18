use gsm_domain::settings::{Field, Kind};
pub fn schema() -> Vec<Field> {
    let mut fields = gsm_domain::settings::paths(true, true);
    fields.extend([
        Field::new(
            "display_name",
            "manager",
            "server",
            "name",
            Kind::Text,
            "Satisfactory",
        ),
        Field::new(
            "port",
            "manager",
            "server",
            "port",
            Kind::Number(1024, 65534),
            "7777",
        ),
        Field::new(
            "messaging_port",
            "manager",
            "server",
            "messaging_port",
            Kind::Number(1024, 65534),
            "8888",
        ),
        Field::new(
            "external_reliable_port",
            "manager",
            "server",
            "external_reliable_port",
            Kind::Number(0, 65535),
            "0",
        ),
        Field::new(
            "api_password",
            "manager",
            "server",
            "admin_password",
            Kind::Secret,
            "",
        ),
        Field::new(
            "api_token",
            "manager",
            "manager",
            "api_token",
            Kind::Secret,
            "",
        ),
    ]);
    fields
}
pub fn creation_schema() -> Vec<Field> {
    schema()
        .into_iter()
        .filter(|f| !["api_password", "api_token"].contains(&f.id))
        .collect()
}
pub fn create(values: &[(String, String)]) -> Result<gsm_domain::settings::Documents, String> {
    let mut seed = gsm_domain::settings::Documents::new();
    seed.insert(
        "manager".into(),
        "[integration]\ninitial_setup=true\n".into(),
    );
    gsm_domain::settings::new_documents(&creation_schema(), seed, values)
}
