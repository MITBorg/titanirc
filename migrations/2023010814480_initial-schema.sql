CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username VARCHAR(255) NOT NULL,
    password VARCHAR(255) NOT NULL
);

CREATE UNIQUE INDEX users_username ON users(username);

CREATE TABLE user_nicks (
    nick VARCHAR(255) NOT NULL PRIMARY KEY,
    user INTEGER NOT NULL,
    FOREIGN KEY(user) REFERENCES users(id)
);

CREATE TABLE channels (
    id INTEGER PRIMARY KEY,
    name VARCHAR(255) NOT NULL
);

CREATE UNIQUE INDEX channel_name ON channels(name);

CREATE TABLE channel_messages (
      channel INT NOT NULL,
      timestamp INT NOT NULL,
      sender VARCHAR(255),
      message VARCHAR(255),
      kind SMALLINT NOT NULL,
      FOREIGN KEY(channel) REFERENCES channels(id),
      PRIMARY KEY(channel, timestamp)
);

CREATE TABLE channel_users (
    channel INT NOT NULL,
    user INT NOT NULL,
    permissions INT NOT NULL DEFAULT 0,
    in_channel BOOLEAN DEFAULT false,
    last_seen_message_timestamp INT,
    FOREIGN KEY(user) REFERENCES users(id),
    FOREIGN KEY(channel) REFERENCES channels(id),
    -- FOREIGN KEY(channel, last_seen_message_timestamp) REFERENCES channel_messages(channel, timestamp)
    PRIMARY KEY(channel, user)
);

CREATE TABLE private_messages (
    timestamp INT NOT NULL PRIMARY KEY,
    sender VARCHAR(255) NOT NULL,
    receiver INT NOT NULL,
    message VARCHAR(255) NOT NULL,
    kind SMALLINT NOT NULL,
    FOREIGN KEY(receiver) REFERENCES users(id)
);

CREATE INDEX private_messages_receiver ON private_messages(receiver);
