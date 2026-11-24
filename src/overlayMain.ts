import { mount } from "svelte";
import NotesOverlay from "./NotesOverlay.svelte";
import "./styles/tokens.css";
import "./styles/base.css";

// The gaming notes overlay's window (ADR-052, overlay.html): its own small
// page, dark whatever the dashboard's theme, since it sits over games.

const target = document.getElementById("app");
if (!target) {
  throw new Error("Missing #app mount element in overlay.html");
}
document.documentElement.dataset["theme"] = "dark";

export default mount(NotesOverlay, { target });
