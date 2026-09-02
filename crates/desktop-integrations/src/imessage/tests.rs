use super::*;

fn fixture() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = Connection::open(dir.path().join("chat.db")).unwrap();
    db.execute_batch("CREATE TABLE chat (chat_identifier TEXT, service_name TEXT);
        CREATE TABLE handle (id TEXT);
        CREATE TABLE chat_handle_join (chat_id INTEGER, handle_id INTEGER);
        CREATE TABLE message (text TEXT, is_from_me INTEGER, date INTEGER, service TEXT, destination_caller_id TEXT);
        CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);
        INSERT INTO chat VALUES ('me@example.test','iMessage'),('friend@example.test','iMessage'),('group','iMessage'),('me@example.test','SMS');
        INSERT INTO handle VALUES ('me@example.test'),('friend@example.test');
        INSERT INTO chat_handle_join VALUES (1,1),(2,2),(3,1),(3,2),(4,1);
        INSERT INTO message VALUES ('my text',1,1000000000,'iMessage','me@example.test'),('private friend text',0,2000000000,'iMessage',NULL);
        INSERT INTO chat_message_join VALUES (1,1),(2,2);").unwrap();
    (dir, db)
}

#[test]
fn only_one_to_one_self_imessage_chats_are_discoverable() {
    let (_dir, db) = fixture();
    let chats = conversations(&db).unwrap();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].id, "1");
    let messages = read(&db, &chats[0]).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "my text");
    assert_eq!(messages[0].unix_seconds, 978307201);
    for id in ["2", "3", "4", "1 OR 1=1"] {
        assert!(read(
            &db,
            &Conversation {
                id: id.into(),
                self_address: "me@example.test".into()
            }
        )
        .is_err());
    }
    assert!(read(
        &db,
        &Conversation {
            id: "1".into(),
            self_address: "friend@example.test".into()
        }
    )
    .is_err());
}

#[test]
fn bounded_plain_text_and_read_only_connection_do_not_touch_other_content() {
    let (dir, db) = fixture();
    for n in 0..60 {
        db.execute(
            "INSERT INTO message VALUES (?1,1,1000000000,'iMessage','me@example.test')",
            ["x".repeat(5000)],
        )
        .unwrap();
        db.execute("INSERT INTO chat_message_join VALUES (1,?1)", [n + 3])
            .unwrap();
    }
    db.execute("UPDATE message SET text=NULL WHERE ROWID=62", [])
        .unwrap();
    drop(db);
    let db = open_database(&dir.path().join("chat.db")).unwrap();
    assert!(db.execute("DELETE FROM message", []).is_err());
    let messages = read(&db, &conversations(&db).unwrap()[0]).unwrap();
    assert_eq!(messages.len(), 50);
    assert_eq!(messages[0].id, "13");
    assert_eq!(messages[0].text.len(), 4000);
    assert_eq!(messages[49].text, "[Non-plain-text message omitted]");
}

#[test]
fn missing_database_is_not_created_and_schema_changes_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.db");
    assert!(open_database(&path).is_err());
    assert!(!path.exists());
    let db = Connection::open_in_memory().unwrap();
    assert!(conversations(&db).is_err());
}
