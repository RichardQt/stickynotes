import sqlite3, os, glob

# Search all plausible tags.db locations
candidates = [
    os.path.expandvars(r"%LOCALAPPDATA%\stickynotes\tags.db"),
    os.path.join(os.path.expanduser("~"), ".local", "share", "stickynotes", "tags.db"),
]
for c in glob.glob(os.path.expandvars(r"%LOCALAPPDATA%\stickynotes\*.db")) + candidates:
    if os.path.exists(c):
        size = os.path.getsize(c)
        mtime = os.path.getmtime(c)
        conn = sqlite3.connect(c)
        cur = conn.cursor()
        try:
            tags = cur.execute("SELECT id, name FROM tag").fetchall()
            links = cur.execute("SELECT note_guid, tag_id FROM note_tag").fetchall()
        except sqlite3.OperationalError:
            tags, links = ("(no tables)",)
        from datetime import datetime
        print(f"{c} | {size}B | {datetime.fromtimestamp(mtime):%H:%M:%S} | tags={tags} links={links}")
        conn.close()
