import { mount } from "svelte";
import App from "./App.svelte";
import "./styles/tokens.css";
import "./styles/base.css";
import { applyTheme, applyWindowTheme, loadTheme } from "./theme";

const target = document.getElementById("app");
if (!target) {
  throw new Error("Missing #app mount element in index.html");
}

// Before the first paint, so the page never flashes the other theme.
const theme = loadTheme();
applyTheme(theme);
if (theme !== "system") {
  void applyWindowTheme(theme);
}

export default mount(App, { target });
