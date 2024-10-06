const { register, isRegistered, unregister, unregisterAll } = window.__TAURI__.globalShortcut;
const { invoke, addEventListener } = window.__TAURI__.core;
const {getCurrentWindow } = window.__TAURI__.window;

let greetInputEl;
let greetMsgEl;

async function greet() {
  // Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
  greetMsgEl.textContent = await invoke("greet", { name: greetInputEl.value });
}

window.addEventListener("DOMContentLoaded", () => {
  greetInputEl = document.querySelector("#greet-input");
  greetMsgEl = document.querySelector("#greet-msg");
  document.querySelector("#greet-form").addEventListener("submit", (e) => {
    e.preventDefault();
    greet();
  });
});


window.__TAURI__.event.listen("log", (event) => {
   if (document.querySelector("#log-view")) {
    document.querySelector("#log-view").textContent = event.data;
   }
});

//listen to ctrl+;
await register('Control+;', () => {
  console.log('Shortcut triggered');
  toggleWindow();
});


async function toggleWindow() {
  const mainWindow = getCurrentWindow();
  if (mainWindow) {
    mainWindow.isVisible() ? mainWindow.hide() : mainWindow.show();
  } else {
    const newWindow = await createWindow({
      url: 'index.html',
      title: 'New Window',
      width: 800,
      height: 600,
    });
    newWindow.show();
  }
}
