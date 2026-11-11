use std::path::Path;

use rusqlite::Connection;

/// Each entry runs once, in order. Never edit a shipped entry; append a new one.
const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    avatar_hue    INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE communities (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    owner_id   TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);

CREATE TABLE members (
    community_id TEXT NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at    INTEGER NOT NULL,
    PRIMARY KEY (community_id, user_id)
);
CREATE INDEX members_user ON members(user_id);

CREATE TABLE channels (
    id           TEXT PRIMARY KEY,
    community_id TEXT NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    topic        TEXT NOT NULL DEFAULT '',
    position     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL
);
CREATE INDEX channels_community ON channels(community_id, position);

CREATE TABLE conversations (
    id              TEXT PRIMARY KEY,
    user_a          TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_b          TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      INTEGER NOT NULL,
    last_message_at INTEGER NOT NULL,
    UNIQUE (user_a, user_b),
    CHECK (user_a < user_b)
);
CREATE INDEX conversations_b ON conversations(user_b);

CREATE TABLE messages (
    id              TEXT PRIMARY KEY,
    channel_id      TEXT REFERENCES channels(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    author_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content         TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    edited_at       INTEGER,
    CHECK ((channel_id IS NULL) <> (conversation_id IS NULL))
);
CREATE INDEX messages_channel ON messages(channel_id, id);
CREATE INDEX messages_conversation ON messages(conversation_id, id);
"#];

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!("Applied database migration {version}");
    }
    Ok(())
}
