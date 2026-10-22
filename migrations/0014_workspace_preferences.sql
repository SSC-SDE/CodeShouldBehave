CREATE TABLE workspace_preferences (
    principal TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    document TEXT NOT NULL
);
