CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username VARCHAR(255) NOT NULL,
    password VARCHAR(255) NOT NULL
);

CREATE UNIQUE INDEX users_username ON users(username);

CREATE TABLE channels (
    id INTEGER PRIMARY KEY,
    name VARCHAR(255) NOT NULL
);

CREATE UNIQUE INDEX channel_name ON channels(name);

CREATE TABLE channel_messages (
      channel INT NOT NULL,
      idx INT NOT NULL,
      sender VARCHAR(255),
      message VARCHAR(255),
      FOREIGN KEY(channel) REFERENCES channels(id),
      PRIMARY KEY(channel, idx)
);

CREATE TABLE channel_users (
    channel INT NOT NULL,
    user INT NOT NULL,
    permissions INT NOT NULL DEFAULT 0,
    in_channel BOOLEAN DEFAULT false,
    last_seen_message_idx INT,
    FOREIGN KEY(user) REFERENCES users(id),
    FOREIGN KEY(channel) REFERENCES channels(id)
    -- FOREIGN KEY(channel, last_seen_message_idx) REFERENCES channels(channel, idx)
);

CREATE UNIQUE INDEX channel_user ON channel_users(channel, user);
