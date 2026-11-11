-- Paracord schema (MySQL 8 / MariaDB 10.5+).
-- Ids are 22-character time-ordered hex strings, so ordering by id is ordering
-- by creation time. Times are milliseconds since 1970 (BIGINT).

CREATE TABLE users (
    id            VARCHAR(32)  NOT NULL PRIMARY KEY,
    username      VARCHAR(32)  NOT NULL UNIQUE,
    display_name  VARCHAR(64)  NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    avatar_hue    BIGINT       NOT NULL,
    created_at    BIGINT       NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE sessions (
    token_hash CHAR(64)    NOT NULL PRIMARY KEY,
    user_id    VARCHAR(32) NOT NULL,
    created_at BIGINT      NOT NULL,
    expires_at BIGINT      NOT NULL,
    INDEX sessions_user (user_id),
    INDEX sessions_expiry (expires_at),
    CONSTRAINT sessions_user_fk FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE communities (
    id         VARCHAR(32)  NOT NULL PRIMARY KEY,
    name       VARCHAR(100) NOT NULL,
    owner_id   VARCHAR(32)  NOT NULL,
    created_at BIGINT       NOT NULL,
    INDEX communities_created (created_at, id),
    CONSTRAINT communities_owner_fk FOREIGN KEY (owner_id) REFERENCES users(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE members (
    community_id VARCHAR(32) NOT NULL,
    user_id      VARCHAR(32) NOT NULL,
    joined_at    BIGINT      NOT NULL,
    PRIMARY KEY (community_id, user_id),
    INDEX members_user (user_id),
    CONSTRAINT members_community_fk FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE,
    CONSTRAINT members_user_fk FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE channels (
    id           VARCHAR(32)  NOT NULL PRIMARY KEY,
    community_id VARCHAR(32)  NOT NULL,
    name         VARCHAR(64)  NOT NULL,
    topic        VARCHAR(400) NOT NULL DEFAULT '',
    position     BIGINT       NOT NULL,
    created_at   BIGINT       NOT NULL,
    INDEX channels_community (community_id, position),
    CONSTRAINT channels_community_fk FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

-- Direct conversations between two people; user_a sorts before user_b.
CREATE TABLE conversations (
    id              VARCHAR(32) NOT NULL PRIMARY KEY,
    user_a          VARCHAR(32) NOT NULL,
    user_b          VARCHAR(32) NOT NULL,
    created_at      BIGINT      NOT NULL,
    last_message_at BIGINT      NOT NULL,
    UNIQUE KEY conversations_pair (user_a, user_b),
    INDEX conversations_b (user_b),
    CONSTRAINT conversations_a_fk FOREIGN KEY (user_a) REFERENCES users(id) ON DELETE CASCADE,
    CONSTRAINT conversations_b_fk FOREIGN KEY (user_b) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

-- Exactly one of channel_id / conversation_id is set (enforced by the server;
-- MySQL won't allow a CHECK on columns with ON DELETE CASCADE).
CREATE TABLE messages (
    id              VARCHAR(32) NOT NULL PRIMARY KEY,
    channel_id      VARCHAR(32) NULL,
    conversation_id VARCHAR(32) NULL,
    author_id       VARCHAR(32) NOT NULL,
    content         TEXT        NOT NULL,
    created_at      BIGINT      NOT NULL,
    edited_at       BIGINT      NULL,
    INDEX messages_channel (channel_id, id),
    INDEX messages_conversation (conversation_id, id),
    CONSTRAINT messages_channel_fk FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
    CONSTRAINT messages_conversation_fk FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    CONSTRAINT messages_author_fk FOREIGN KEY (author_id) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
