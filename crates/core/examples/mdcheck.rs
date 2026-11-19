fn main() {
    let plum = r"\id=03c89f49-1b4e-4ff3-9ad1-dc0de82119cc\b 示例便签标题（加粗）\b0
\id=a440db62-6558-4c4e-811d-85d20833544a
\id=d9883dd5-4120-46d3-acbb-48438524fc22 普通段落与 \i 斜体文本\i0
\id=13ffb531-b38d-4a36-894c-97431a9b3f49 - 待办事项示例";
    let md = stickynotes_core::markup::plum_to_markdown(plum);
    println!("=== markdown ===");
    println!("{md}");
    println!("=== end ===");
}
