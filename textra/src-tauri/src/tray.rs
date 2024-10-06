use anyhow::Result;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Manager, Wry,
};

use crate::{config};

/// 初始化托盘菜单
///
/// Initialize tray menu
pub fn init(app: &AppHandle) -> Result<()> {
    let menu = menu(app)?;
    let _ = TrayIconBuilder::with_id("menu")
        .tooltip("Tran")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
 
        .build(app);
    Ok(())
}

fn menu(handle: &AppHandle) -> Result<Menu<Wry>> {
     match Menu::new(handle) {
        Ok(mut menu) => {
             let exit = MenuItem::with_id(handle, "exit", "Exit", true, None::<&str>)?;
             menu.append(&exit);
            Ok(menu)
        }
        Err(e) => Err(anyhow::anyhow!("Failed to create menu: {}", e)),
     }
}

fn fresh(app: &AppHandle) {
    let _ = app
        .tray_by_id("menu")
        .unwrap()
        .set_menu(Some(menu(app).unwrap()));
}

 