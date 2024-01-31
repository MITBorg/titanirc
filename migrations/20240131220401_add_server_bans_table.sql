CREATE TABLE server_bans (
    mask VARCHAR(255) NOT NULL,
    requester INT NOT NULL,
    reason VARCHAR(255) NOT NULL,
    created_timestamp INT NOT NULL,
    expires_timestamp INT,
    FOREIGN KEY(requester) REFERENCES users,
    PRIMARY KEY(mask)
);
