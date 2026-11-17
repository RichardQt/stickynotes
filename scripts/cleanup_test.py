import sqlite3, os, time

path = os.path.expandvars(
    r"%LOCALAPPDATA%\Packages\Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe\LocalState\plum.sqlite"
)
conn = sqlite3.connect(path)
cur = conn.cursor()

# .NET ticks for now
epoch = 621355968000000000
now = epoch + int(time.time() * 10_000_000)

ids = [
    "699f8a6f-21b9-4202-a146-267b57e96586",  # spike-insert-test
    "5693de6c",  # empty note (find full id first)
]
# Find the empty note's full id
row = cur.execute(
    "SELECT Id FROM Note WHERE DeletedAt IS NULL AND (Text IS NULL OR Text = '') "
    "ORDER BY UpdatedAt DESC LIMIT 1"
).fetchone()
if row:
    ids[1] = row[0]

for id_ in ids:
    cur.execute("UPDATE Note SET DeletedAt = ?, UpdatedAt = ? WHERE Id = ?", (now, now, id_))
    print(f"soft-deleted {id_[:8]} (rows: {cur.rowcount})")
conn.commit()
conn.close()
print("done — launch built-in Sticky Notes to push the deletions to the cloud")
