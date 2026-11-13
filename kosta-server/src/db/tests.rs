// kosta-server/src/db/tests.rs
//! Тесты слоя БД: хелпер TempDb живёт рядом с тестами.

use super::*;

/// Временная БД в файле: пул соединений с `:memory:` дал бы каждому
/// соединению свою пустую базу, поэтому используем файл во временном каталоге.
struct TempDb {
    db: Database,
    path: std::path::PathBuf,
}

impl TempDb {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("kosta_test_{}.db", rand::random::<u64>()));
        let secrets = Arc::new(SecretsBox::from_key([0x42u8; 32]));
        let db = Database::new(path.to_str().expect("temp path is utf-8"), secrets)
            .expect("не удалось открыть тестовую БД");
        TempDb { db, path }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", self.path.display(), suffix));
        }
    }
}

#[test]
fn token_roundtrip_and_revoke() {
    let t = TempDb::new();
    let user_id = t.db.create_user("+1000", "pass").unwrap();

    let token = t.db.create_auth_token(user_id).unwrap();
    assert_eq!(token.len(), 64, "токен — 32 случайных байта в hex");
    assert_eq!(t.db.user_id_by_token(&token).unwrap(), Some(user_id));

    // Отозванный токен больше не принимается
    t.db.revoke_token(&token).unwrap();
    assert_eq!(t.db.user_id_by_token(&token).unwrap(), None);

    // Чужой/мусорный токен — тоже нет
    assert_eq!(t.db.user_id_by_token("deadbeef").unwrap(), None);
    assert_eq!(t.db.user_id_by_token("").unwrap(), None);
}

#[test]
fn expired_token_is_rejected_and_purged() {
    let t = TempDb::new();
    let user_id = t.db.create_user("+1001", "pass").unwrap();
    let token = t.db.create_auth_token(user_id).unwrap();

    // Просрочиваем токен вручную
    t.db.get_conn()
        .unwrap()
        .execute(
            "UPDATE auth_tokens SET expires_at = ?1",
            rusqlite::params![now_secs() - 1],
        )
        .unwrap();

    assert_eq!(t.db.user_id_by_token(&token).unwrap(), None);
    assert_eq!(t.db.purge_expired_tokens().unwrap(), 1);
}

#[test]
fn wrong_password_does_not_create_new_user() {
    let t = TempDb::new();
    let user_id = t.db.create_user("+1002", "correct").unwrap();

    // Пароль неверный: authenticate возвращает None, но пользователь существует,
    // значит логин-хендлер обязан ответить 401, а не регистрировать нового.
    assert_eq!(t.db.authenticate("+1002", "wrong").unwrap(), None);
    assert_eq!(t.db.get_user_id_by_phone("+1002").unwrap(), Some(user_id));
    assert_eq!(
        t.db.authenticate("+1002", "correct").unwrap(),
        Some(user_id)
    );
}

#[test]
fn foreign_keys_are_enforced() {
    let t = TempDb::new();
    let user_id = t.db.create_user("+2000", "pass").unwrap();

    // Участник с несуществующим user_id должен отвергаться БД, а не создавать мусор.
    let conn = t.db.get_conn().unwrap();
    let orphan_participant = conn.execute(
        "INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (1, 999999, 0)",
        [],
    );
    assert!(
        orphan_participant.is_err(),
        "PRAGMA foreign_keys=ON должен запрещать участников-призраков"
    );

    // Сообщение в несуществующий диалог тоже не должно сохраняться.
    assert!(t
        .db
        .save_message(999999, user_id, "text", 0, true, None, None)
        .is_err());
}

#[test]
fn private_dialog_with_self_is_rejected() {
    let t = TempDb::new();
    let user_id = t.db.create_user("+2001", "pass").unwrap();
    assert!(t.db.create_private_dialog(user_id, user_id).is_err());
}

#[test]
fn private_dialog_is_reused_and_has_exactly_two_participants() {
    let t = TempDb::new();
    let u1 = t.db.create_user("+2002", "pass").unwrap();
    let u2 = t.db.create_user("+2003", "pass").unwrap();

    let first = t.db.create_private_dialog(u1, u2).unwrap();
    let second = t.db.create_private_dialog(u1, u2).unwrap();
    assert_eq!(
        first, second,
        "повторный вызов должен вернуть тот же диалог"
    );

    let mut participants = t.db.get_dialog_participants(first).unwrap();
    participants.sort();
    assert_eq!(participants, vec![u1, u2]);
    assert_eq!(t.db.get_dialog_type(first).unwrap(), Some(0));
}

#[test]
fn group_type_title_and_admin_roles() {
    let t = TempDb::new();
    let creator = t.db.create_user("+2004", "pass").unwrap();
    let member = t.db.create_user("+2005", "pass").unwrap();

    let dialog_id =
        t.db.create_group_dialog("  Тестовая группа  ", creator, &[member])
            .unwrap();

    let meta = t.db.get_dialog_meta(dialog_id, creator).unwrap().unwrap();
    assert!(
        meta.is_group,
        "тип берётся из dialogs.type, а не из числа участников"
    );
    assert_eq!(
        meta.title, "Тестовая группа",
        "название группы должно сохраняться"
    );
    assert_eq!(meta.participants.len(), 2);
    assert!(t.db.is_dialog_admin(dialog_id, creator).unwrap());
    assert!(!t.db.is_dialog_admin(dialog_id, member).unwrap());

    t.db.remove_participant(dialog_id, member).unwrap();
    let meta = t.db.get_dialog_meta(dialog_id, creator).unwrap().unwrap();
    assert_eq!(meta.participants, vec![creator]);
    // Даже с одним участником группа остаётся группой.
    assert!(meta.is_group);
}

#[test]
fn user_chats_resolve_title_and_last_message() {
    let t = TempDb::new();
    let u1 = t.db.create_user("+2006", "pass").unwrap();
    let u2 = t.db.create_user("+2007", "pass").unwrap();
    let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();
    t.db.save_message(dialog_id, u2, "привет", 100, false, None, None)
        .unwrap();
    t.db.save_message(dialog_id, u1, "ответ", 200, true, None, None)
        .unwrap();

    let chats = t.db.get_user_chats(u1).unwrap();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].id, dialog_id);
    assert!(!chats[0].is_group);
    assert_eq!(
        chats[0].title, "+2007",
        "личный диалог называется по телефону"
    );
    assert_eq!(chats[0].last_message.as_deref(), Some("ответ"));
    assert_eq!(chats[0].last_timestamp, Some(200));

    let group_id = t.db.create_group_dialog("Друзья", u1, &[u2]).unwrap();
    let chats = t.db.get_user_chats(u1).unwrap();
    let group = chats
        .iter()
        .find(|c| c.id == group_id)
        .expect("группа в списке");
    assert!(group.is_group);
    assert_eq!(group.title, "Друзья");
}

#[test]
fn duplicate_registration_is_an_error_not_a_panic() {
    let t = TempDb::new();
    let id = t.db.create_user("+2009", "pass").unwrap();
    // Гонка/повтор: второй INSERT OR IGNORE не должен давать 500 из-за UNIQUE.
    let err = t.db.create_user("+2009", "other").unwrap_err();
    assert!(format!("{}", err).contains("already registered"));
    assert_eq!(t.db.get_user_id_by_phone("+2009").unwrap(), Some(id));
}

#[test]
fn migration_adds_new_columns_to_old_database() {
    let path =
        std::env::temp_dir().join(format!("kosta_migration_test_{}.db", rand::random::<u64>()));
    // Старая схема: без is_admin и last_used_at.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
                r"
                CREATE TABLE users (id INTEGER PRIMARY KEY, phone TEXT UNIQUE NOT NULL,
                                    password_hash TEXT NOT NULL, status INTEGER NOT NULL DEFAULT 0,
                                    last_seen INTEGER);
                CREATE TABLE dialogs (id INTEGER PRIMARY KEY, type INTEGER NOT NULL,
                    title TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL);
                CREATE TABLE dialog_participants (dialog_id INTEGER, user_id INTEGER,
                    joined_at INTEGER NOT NULL, PRIMARY KEY (dialog_id, user_id));
                CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, dialog_id INTEGER NOT NULL,
                    sender_id INTEGER NOT NULL, text TEXT NOT NULL, timestamp INTEGER NOT NULL,
                    is_outgoing INTEGER NOT NULL, read_status INTEGER NOT NULL DEFAULT 0,
                    delivered_status INTEGER NOT NULL DEFAULT 0);
                INSERT INTO users (id, phone, password_hash) VALUES (0, 'system', '');
                INSERT INTO users (id, phone, password_hash) VALUES (1, '+3000', 'x');
                INSERT INTO users (id, phone, password_hash) VALUES (2, '+3001', 'x');
                INSERT INTO dialogs (id, type, title, created_at) VALUES (10, 1, 'Old group', 0);
                INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (10, 1, 100);
                INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (10, 2, 200);
                ",
            )
            .unwrap();
    }

    let secrets = Arc::new(SecretsBox::from_key([0x42u8; 32]));
    let db = Database::new(path.to_str().unwrap(), secrets).expect("миграция старой БД");
    // Первый вступивший в старую группу становится админом.
    assert!(db.is_dialog_admin(10, 1).unwrap());
    assert!(!db.is_dialog_admin(10, 2).unwrap());
    // Media-колонки добавляются в старую БД, и через них уже можно
    // сохранять/читать медиа-сообщение.
    let media_id = db
        .save_media_message(
            10,
            1,
            "подпись",
            0,
            "10/0123456789abcdef0123456789abcdef.png",
            "image/png",
            3,
            "a.png",
        )
        .expect("media-колонки должны появиться в старой БД");
    assert_eq!(
        db.get_media_message(media_id)
            .unwrap()
            .expect("медиа-сообщение читается")
            .media_key,
        "10/0123456789abcdef0123456789abcdef.png"
    );
    drop(db);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

#[test]
fn unread_count_counts_only_incoming_undeleted_unread() {
    let t = TempDb::new();
    let u1 = t.db.create_user("+2100", "pass").unwrap();
    let u2 = t.db.create_user("+2101", "pass").unwrap();
    let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();

    // Три входящих, одно своё и одно входящее «удалённое».
    let already_read =
        t.db.save_message(dialog_id, u2, "прочитано", 100, false, None, None)
            .unwrap();
    t.db.save_message(dialog_id, u2, "новое 1", 101, false, None, None)
        .unwrap();
    t.db.save_message(dialog_id, u2, "новое 2", 102, false, None, None)
        .unwrap();
    t.db.save_message(dialog_id, u1, "моё", 103, true, None, None)
        .unwrap();
    let deleted =
        t.db.save_message(dialog_id, u2, "удалено", 104, false, None, None)
            .unwrap();

    // Прочитанность и удаление — руками: публичных методов для них нет.
    let conn = t.db.get_conn().unwrap();
    conn.execute(
        "UPDATE messages SET read_status = 1 WHERE id = ?1",
        rusqlite::params![already_read],
    )
    .unwrap();
    conn.execute(
        "UPDATE messages SET deleted = 1 WHERE id = ?1",
        rusqlite::params![deleted],
    )
    .unwrap();
    drop(conn);

    let row = t.db.get_chat_row(u1, dialog_id).unwrap().unwrap();
    assert_eq!(row.unread_count, 2, "только два входящих непрочитанных");
    assert_eq!(row.other_user_id, Some(u2));
    assert_eq!(row.title, "+2101");
    assert!(!row.is_online, "без строки user_status собеседник offline");

    t.db.set_user_status(u2, 0).unwrap();
    assert!(
        t.db.get_chat_row(u1, dialog_id).unwrap().unwrap().is_online,
        "status 0 — онлайн"
    );
    t.db.set_user_status(u2, 1).unwrap();
    assert!(
        !t.db.get_chat_row(u1, dialog_id).unwrap().unwrap().is_online,
        "status 1 — офлайн"
    );

    // Для второго участника собеседник — первый.
    let peer = t.db.get_chat_row(u2, dialog_id).unwrap().unwrap();
    assert_eq!(peer.other_user_id, Some(u1));
    assert_eq!(peer.unread_count, 1, "непрочитанное входящее от u1");

    // У группы нет «собеседника» и статуса.
    let group_id = t.db.create_group_dialog("Группа", u1, &[u2]).unwrap();
    let group = t.db.get_chat_row(u1, group_id).unwrap().unwrap();
    assert!(group.is_group);
    assert_eq!(group.other_user_id, None);
    assert!(!group.is_online);
    assert_eq!(group.unread_count, 0);

    // Постороннему карточка не положена.
    let stranger = t.db.create_user("+2102", "pass").unwrap();
    assert!(t.db.get_chat_row(stranger, dialog_id).unwrap().is_none());
    assert!(t.db.get_chat_row(stranger, group_id).unwrap().is_none());
}

#[test]
fn mark_dialog_read_marks_only_incoming_and_is_idempotent() {
    let t = TempDb::new();
    let u1 = t.db.create_user("+2103", "pass").unwrap();
    let u2 = t.db.create_user("+2104", "pass").unwrap();
    let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();

    t.db.save_message(dialog_id, u2, "входящее 1", 100, false, None, None)
        .unwrap();
    t.db.save_message(dialog_id, u2, "входящее 2", 101, false, None, None)
        .unwrap();
    t.db.save_message(dialog_id, u1, "исходящее", 102, true, None, None)
        .unwrap();

    // Для u1 входящих непрочитанных два, исходящее не трогаем.
    assert_eq!(t.db.mark_dialog_read(dialog_id, u1).unwrap(), 2);
    assert_eq!(
        t.db.mark_dialog_read(dialog_id, u1).unwrap(),
        0,
        "повторный вызов нечего помечать"
    );
    assert_eq!(
        t.db.get_chat_row(u1, dialog_id)
            .unwrap()
            .unwrap()
            .unread_count,
        0
    );

    // Для u2 входящее — одно (сообщение u1), остальное его собственное.
    assert_eq!(t.db.mark_dialog_read(dialog_id, u2).unwrap(), 1);
    assert_eq!(t.db.mark_dialog_read(dialog_id, u2).unwrap(), 0);
}

#[test]
fn detailed_participants_include_phone_role_and_online() {
    let t = TempDb::new();
    let creator = t.db.create_user("+2105", "pass").unwrap();
    let member = t.db.create_user("+2106", "pass").unwrap();
    let offline = t.db.create_user("+2107", "pass").unwrap();
    let group_id =
        t.db.create_group_dialog("Команда", creator, &[member, offline])
            .unwrap();
    t.db.set_user_status(creator, 1).unwrap();
    t.db.set_user_status(member, 0).unwrap();

    let participants = t.db.get_dialog_participants_detailed(group_id).unwrap();
    assert_eq!(participants.len(), 3);
    let by_id = |id: i64| {
        participants
            .iter()
            .find(|p| p.user_id == id)
            .expect("участник должен быть в списке")
    };
    assert!(by_id(creator).is_admin, "создатель — администратор");
    assert_eq!(by_id(creator).phone, "+2105");
    assert!(!by_id(creator).is_online, "status 1 — офлайн");
    assert!(!by_id(member).is_admin);
    assert!(by_id(member).is_online, "status 0 — онлайн");
    assert!(
        !by_id(offline).is_online,
        "отсутствие строки user_status — offline"
    );

    // Личный диалог тоже отдаёт участников (для карточки чата).
    let private_id = t.db.create_private_dialog(member, offline).unwrap();
    let private = t.db.get_dialog_participants_detailed(private_id).unwrap();
    assert_eq!(private.len(), 2);
    assert!(private.iter().all(|p| !p.is_admin));
}

#[test]
fn rename_dialog_updates_title_and_reports_missing_dialog() {
    let t = TempDb::new();
    let creator = t.db.create_user("+2109", "pass").unwrap();
    let group_id = t.db.create_group_dialog("Старое", creator, &[]).unwrap();

    t.db.rename_dialog(group_id, "Новое").unwrap();
    let meta = t.db.get_dialog_meta(group_id, creator).unwrap().unwrap();
    assert_eq!(meta.title, "Новое");
    assert!(t.db.rename_dialog(999_999, "x").is_err());
}

#[test]
fn lookup_by_phone_is_exact_and_profile_reports_online() {
    let t = TempDb::new();
    let user_id = t.db.create_user("+2110", "pass").unwrap();

    assert_eq!(
        t.db.find_user_by_phone("+2110").unwrap(),
        Some((user_id, "+2110".to_string()))
    );
    assert_eq!(t.db.find_user_by_phone("+2111").unwrap(), None);
    assert_eq!(
        t.db.find_user_by_phone("2110").unwrap(),
        None,
        "номер сверяется точно, без подстрок"
    );

    let profile = t.db.get_user_profile(user_id).unwrap().expect("профиль");
    assert_eq!(profile.phone, "+2110");
    assert!(!profile.is_online, "нет строки user_status — offline");
    t.db.set_user_status(user_id, 0).unwrap();
    let profile = t.db.get_user_profile(user_id).unwrap().expect("профиль");
    assert!(profile.is_online);
    t.db.set_user_status(user_id, 3).unwrap();
    let profile = t.db.get_user_profile(user_id).unwrap().expect("профиль");
    assert!(!profile.is_online, "любой статус кроме 0 — offline");
    assert!(t.db.get_user_profile(999_999).unwrap().is_none());
}

#[test]
fn media_message_roundtrip_in_history_and_by_id() {
    let t = TempDb::new();
    let u1 = t.db.create_user("+2200", "pass").unwrap();
    let u2 = t.db.create_user("+2201", "pass").unwrap();
    let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();
    let media_key = format!("{dialog_id}/0123456789abcdef0123456789abcdef.png");

    let message_id =
        t.db.save_media_message(
            dialog_id,
            u1,
            "подпись",
            1_234,
            &media_key,
            "image/png",
            5,
            "photo.png",
        )
        .unwrap();

    let history = t.db.get_history(dialog_id, 0, 50).unwrap();
    assert_eq!(history.len(), 1);
    let row = &history[0];
    assert_eq!(row.id, message_id);
    assert_eq!(row.sender_id, u1);
    assert_eq!(row.text, "подпись");
    assert_eq!(row.timestamp, 1_234);
    assert!(row.is_outgoing);
    assert!(!row.read);
    assert!(!row.delivered);
    assert_eq!(row.media_path.as_deref(), Some(media_key.as_str()));
    assert_eq!(row.media_type.as_deref(), Some("image/png"));
    assert_eq!(row.media_size, Some(5));
    assert_eq!(row.media_name.as_deref(), Some("photo.png"));
    assert!(row.is_media(), "media_path IS NOT NULL => is_media");

    let media =
        t.db.get_media_message(message_id)
            .unwrap()
            .expect("медиа-сообщение должно читаться по id");
    assert_eq!(media.dialog_id, dialog_id);
    assert_eq!(media.media_key, media_key);
    assert_eq!(media.mime, "image/png");
    assert_eq!(media.size, Some(5));
    assert_eq!(media.name.as_deref(), Some("photo.png"));

    // Сообщение другого диалога/несуществующее — None.
    assert!(t.db.get_media_message(999_999).unwrap().is_none());
}

#[test]
fn plain_message_has_no_media_metadata_and_cannot_be_downloaded() {
    let t = TempDb::new();
    let u1 = t.db.create_user("+2202", "pass").unwrap();
    let u2 = t.db.create_user("+2203", "pass").unwrap();
    let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();

    let message_id =
        t.db.save_message(dialog_id, u1, "просто текст", 10, true, None, None)
            .unwrap();

    let row = t.db.get_history(dialog_id, 0, 10).unwrap().remove(0);
    assert!(!row.is_media());
    assert!(row.media_path.is_none());
    assert!(row.media_type.is_none());
    assert!(row.media_size.is_none());
    assert!(row.media_name.is_none());
    assert!(
        t.db.get_media_message(message_id).unwrap().is_none(),
        "обычный текст не скачивается как медиа"
    );
}

#[test]
fn deleted_media_message_disappears_from_history_and_download() {
    let t = TempDb::new();
    let u1 = t.db.create_user("+2204", "pass").unwrap();
    let u2 = t.db.create_user("+2205", "pass").unwrap();
    let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();
    let message_id =
        t.db.save_media_message(
            dialog_id,
            u1,
            "",
            10,
            &format!("{dialog_id}/0123456789abcdef0123456789abcdef.png"),
            "image/png",
            1,
            "a.png",
        )
        .unwrap();

    t.db.get_conn()
        .unwrap()
        .execute(
            "UPDATE messages SET deleted = 1 WHERE id = ?1",
            rusqlite::params![message_id],
        )
        .unwrap();

    assert!(t.db.get_history(dialog_id, 0, 10).unwrap().is_empty());
    assert!(t.db.get_media_message(message_id).unwrap().is_none());
}
