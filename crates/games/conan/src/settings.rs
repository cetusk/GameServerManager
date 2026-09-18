use gsm_domain::settings::{Field, Kind};
pub fn schema() -> Vec<Field> {
    let mut fields = gsm_domain::settings::paths(false, false);
    fields.extend([
        Field::new(
            "port",
            "manager",
            "server",
            "game_port",
            Kind::Number(1024, 65534),
            "7777",
        ),
        Field::new(
            "query_port",
            "manager",
            "server",
            "query_port",
            Kind::Number(1024, 65534),
            "27015",
        ),
        Field::new(
            "rcon_port",
            "manager",
            "server",
            "rcon_port",
            Kind::Number(1024, 65534),
            "25575",
        ),
        Field::new(
            "rcon_enabled",
            "manager",
            "server",
            "rcon_enabled",
            Kind::Bool,
            "true",
        ),
        Field::new(
            "max_players",
            "manager",
            "server",
            "max_players",
            Kind::Number(1, 100),
            "10",
        ),
        Field::new(
            "name",
            "Engine.ini",
            "@ini:OnlineSubsystem",
            "ServerName",
            Kind::Text,
            "Conan Exiles",
        ),
        Field::new(
            "password",
            "ServerSettings.ini",
            "@ini:ServerSettings",
            "ServerPassword",
            Kind::Secret,
            "",
        ),
        Field::new(
            "admin_password",
            "ServerSettings.ini",
            "@ini:ServerSettings",
            "AdminPassword",
            Kind::Secret,
            "",
        ),
        Field::new(
            "rcon_password",
            "Game.ini",
            "@ini:RconPlugin",
            "RconPassword",
            Kind::Secret,
            "",
        ),
    ]);
    fields
}
pub fn creation_schema() -> Vec<Field> {
    schema()
}
pub fn create(values: &[(String, String)]) -> Result<gsm_domain::settings::Documents, String> {
    gsm_domain::settings::new_documents(&creation_schema(), Default::default(), values)
}
pub fn creation_paths(root: &std::path::Path) -> Vec<(&'static str, std::path::PathBuf)> {
    ["Engine.ini", "ServerSettings.ini", "Game.ini"]
        .map(|name| {
            (
                name,
                root.join("ConanSandbox/Saved/Config/WindowsServer")
                    .join(name),
            )
        })
        .into()
}
