//! Game-owned catalog and local runtime plan.
pub mod local;
use gsm_domain::{GameDescriptor, GameId};

pub fn descriptor() -> GameDescriptor {
    GameDescriptor {
        id: GameId::try_from("windrose".to_owned()).expect("static game ID"),
        name: "Windrose",
        description: "招待コード、ワールド ID 整合",
        sample_world: "Island-A",
        backup_scope: "ワールドとゲーム生成 ZIP（クライアントのキャラクターは対象外）",
    }
}

pub mod settings;
