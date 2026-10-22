fn main() {
    // Trocar um ícone precisa regravar o ícone embutido no executável (janela, barra de tarefas): o Tauri
    // não observa a pasta `icons/`, então sem isto o Cargo reaproveitaria o recurso antigo.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
