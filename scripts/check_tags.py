import sqlite3, os

path = os.path.expandvars(r"%LOCALAPPDATA%\stickynotes\tags.db")
conn = sqlite3.connect(path)
cur = conn.cursor()
try:
    tags = cur.execute("SELECT id, name FROM tag").fetchall()
    print("tags:", tags)
    links = cur.execute("SELECT note_guid, tag_id FROM note_tag").fetchall()
    print("links:", links)
except sqlite3.OperationalError as e:
    print("err:", e)
