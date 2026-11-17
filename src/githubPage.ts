// Pages of this repository on GitHub, opened in the browser (ADR-049;
// src-tauri/src/github_page.rs): the feedback and bug report forms and the
// privacy notice. The page names only which one: Rust builds the address,
// so the page never chooses a URL.
import { invoke } from "@tauri-apps/api/core";

export const OPEN_GITHUB_PAGE = "open_github_page";

export type GithubPage = "feedback" | "bug" | "privacy-notice";

/** Opens the page in the browser. Rejects with Rust's message. */
export async function openGithubPage(page: GithubPage): Promise<void> {
  try {
    await invoke(OPEN_GITHUB_PAGE, { page });
  } catch (e) {
    throw new Error(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
  }
}
