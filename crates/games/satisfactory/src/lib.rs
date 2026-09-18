//! Game-owned catalog and local server plan.
pub mod local;
use gsm_domain::{GameDescriptor, GameId};

pub fn descriptor() -> GameDescriptor {
    GameDescriptor {
        id: GameId::try_from("satisfactory".to_owned()).expect("static game ID"),
        name: "Satisfactory",
        description: "HTTPS API、セッション、セーブ",
        sample_world: "Factory-01",
        backup_scope: "SaveGames、blueprints、設定",
    }
}

pub mod settings;
