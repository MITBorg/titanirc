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

CREATE TABLE channel_users (
    channel INT NOT NULL,
    user INT NOT NULL,
    permissions INT NOT NULL DEFAULT 0,
    in_channel BOOLEAN DEFAULT false,
    FOREIGN KEY(user) REFERENCES users(id),
    FOREIGN KEY(channel) REFERENCES channels(id)
);

CREATE UNIQUE INDEX channel_user ON channel_users(channel, user);
