// Evita a janela de console extra no Windows em release. NÃO REMOVER.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    atlas_desktop_lib::run();
}
