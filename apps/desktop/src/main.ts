import "@fontsource/ibm-plex-sans/latin-400.css";
import "@fontsource/ibm-plex-sans/latin-500.css";
import "@fontsource/ibm-plex-sans/latin-600.css";
import "@fontsource/ibm-plex-sans/latin-ext-400.css";
import "@fontsource/ibm-plex-sans/latin-ext-500.css";
import "@fontsource/ibm-plex-sans/latin-ext-600.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-ext-400.css";
import "./styles/atlas.css";
import "./styles/app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { start } from "./lib/stores/app.svelte";
import { call } from "./lib/api/client";

// The window is drawn first; the state arrives with the first call and then by updates.
mount(App, { target: document.getElementById("app")! });
void start();
// Two frames after the mount the shell is on screen: the number the build is compared by.
requestAnimationFrame(() => requestAnimationFrame(() => void call("log.first_paint").catch(() => {})));
