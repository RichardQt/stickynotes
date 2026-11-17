fn main() {
    let plum = r"\id=03c89f49-1b4e-4ff3-9ad1-dc0de82119cc\b mistral.ai 学生套餐（1201过期）\b0
\id=a440db62-6558-4c4e-811d-85d20833544a
\id=d9883dd5-4120-46d3-acbb-48438524fc22 账号：1024233707@njupt.edu.cn
\id=13ffb531-b38d-4a36-894c-97431a9b3f49 密码：asdfghjkl!@123";
    let md = stickynotes_core::markup::plum_to_markdown(plum);
    println!("=== markdown ===");
    println!("{}", md);
    println!("=== end ===");
}
