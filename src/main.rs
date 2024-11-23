#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    unused_assignments,


)]

use std::env;

use minimo::*;
use installer::*;
use textra::*;
use textra::config::*;
use textra::keyboard::*;
use anyhow::Result;


fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
//if applicaton is launched by double clicking the icon
//we want window to stay open (usually it closes immediately)

    if args.len() == 1 {
         display_help();
         //wait for 2 seconds before closing
         std::thread::sleep(std::time::Duration::from_secs(2));
         return Ok(());
    }


    match args[1].as_str() {
        "run" | "start" => handle_run(),
        "config" | "edit" | "settings" => {
            handle_edit_config().unwrap();
            Ok(())
        }
        "daemon" | "service" => handle_daemon(),
        "stop" | "kill" => handle_stop(),
        "install" | "setup" => handle_install(),
        "uninstall" | "remove" => handle_uninstall(),
        "update" => handle_update(),
        _ => {
            match auto_install() {
                Ok(_) => {
                    display_help();
                    return Ok(());
                },
                Err(e) => {
                    eprintln!("Error: {}", e);
                    display_help();
                    return Ok(());
                }
            }
        }
    }

     
}



fn handle_display_status() {
    if is_service_running() {
        showln!(
            yellow_bold,
            "│ ",
            gray_dim,
            "service: ",
            green_bold,
            "running."
        );
    } else {
        showln!(
            yellow_bold,
            "│ ",
            gray_dim,
            "service: ",
            orange_bold,
            "not running."
        );
    }
    if installer::check_autostart() {
        showln!(
            yellow_bold,
            "│ ",
            gray_dim,
            "autostart: ",
            green_bold,
            "enabled."
        );
    } else {
        showln!(
            yellow_bold,
            "│ ",
            gray_dim,
            "autostart: ",
            orange_bold,
            "disabled."
        );
    }
}

fn display_help() {
    BANNER.show(white_bold);
    divider();
    showln!(
        yellow_bold,
        "┌─ ",
        whitebg,
        " STATUS ",
        yellow_bold,
        " ──────────"
    );
    showln!(yellow_bold, "│ ");
    handle_display_status();
    showln!(yellow_bold, "│ ");
    showln!(yellow_bold, "│ ", whitebg, " HOW TO USE ");
    showln!(yellow_bold, "│ ");
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra run ",
        gray_dim,
        "- Start the Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra stop ",
        gray_dim,
        "- Stop the running Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra install ",
        gray_dim,
        "- Install Textra as a service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra uninstall ",
        gray_dim,
        "- Uninstall the Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra status ",
        gray_dim,
        "- Display the status of the Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra edit ",
        gray_dim,
        "- Edit the Textra configuration file"
    );
    showln!(yellow_bold, "│ ");

    display_config();
}
