//! Cloud-sync end-to-end test: insert a real note, launch the built-in app,
//! wait for it to push the note to the cloud (RemoteId becomes non-null).

use stickynotes_core::{markup, store::PlumStore, sync};

fn main() -> anyhow::Result<()> {
    let store = PlumStore::open()?;
    if sync::is_sticky_notes_running() {
        anyhow::bail!("内置便签正在运行，请先关闭");
    }

    let md = "# 云同步测试\n\n- **来自 gpui-kit 便签应用**\n- 这条笔记由第三方应用直接写入 plum.sqlite\n- 如果内置便签接受它，RemoteId 会被填充并同步到云端";
    let plum = markup::markdown_to_plum(md);
    println!("plum text:\n{plum}\n");

    let id = store.insert_note(&plum, "Green")?;
    println!("inserted: {id}");
    println!("now launching built-in Sticky Notes; it should push this note to the cloud...");
    sync::launch_sticky_notes()?;

    let pushed = sync::wait_until_synced(&store, &id, std::time::Duration::from_secs(90));
    if pushed {
        println!("SUCCESS: RemoteId was filled by the built-in app — the note is syncing to the cloud.");
    } else {
        println!("TIMEOUT: RemoteId still empty after 90s. The note may exist locally but not sync (check the built-in app UI).");
    }
    Ok(())
}
