use gsm_domain::settings::{Field, Kind};
pub fn schema() -> Vec<Field> {
    let mut fields = gsm_domain::settings::paths(false, false);
    fields.extend([
        Field::new(
            "name",
            "ServerDescription.json",
            "@json",
            "ServerName",
            Kind::Text,
            "",
        ),
        Field::new(
            "password",
            "ServerDescription.json",
            "@json",
            "Password",
            Kind::Secret,
            "",
        ),
        Field::new(
            "password_protected",
            "ServerDescription.json",
            "@json",
            "IsPasswordProtected",
            Kind::Bool,
            "false",
        ),
        Field::new(
            "max_players",
            "ServerDescription.json",
            "@json",
            "MaxPlayerCount",
            Kind::Number(1, 8),
            "4",
        ),
    ]);
    fields
}
pub fn creation_schema() -> Vec<Field> {
    gsm_domain::settings::paths(false, false)
}
pub fn create(values: &[(String, String)]) -> Result<gsm_domain::settings::Documents, String> {
    let mut seed = gsm_domain::settings::Documents::new();
    seed.insert(
        "manager".into(),
        "[integration]\ninitial_setup=true\n".into(),
    );
    gsm_domain::settings::new_documents(&creation_schema(), seed, values)
}
