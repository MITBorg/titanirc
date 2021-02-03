CREATE TABLE channels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE
);

CREATE TABLE channel_opers (
    channel_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    level INTEGER NOT NULL,
    FOREIGN KEY (channel_id)
        REFERENCES channels (id),
    FOREIGN KEY (user_id)
        REFERENCES users (id)
);

CREATE TABLE bans (
    mask BINARY PRIMARY KEY,
    expires INTEGER,
    created INTEGER DEFAULT CURRENT_TIMESTAMP,
    revoked BOOLEAN DEFAULT false
);

CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    password BINARY(60),
    op BOOLEAN DEFAULT false
);

CREATE TABLE users_aliases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nick TEXT UNIQUE,
    user_id INT NOT NULL,
    FOREIGN KEY (user_id)
        REFERENCES users (id)
);