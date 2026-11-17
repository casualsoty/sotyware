// The feedback and bug report buttons (ADR-049; src-tauri/src/feedback.rs).
// The page names only which form it wants: Rust builds the GitHub address
// and opens it in the browser, so the page never chooses a URL.
import { invoke } from "@tauri-apps/api/core";

export const OPEN_FEEDBACK_PAGE = "open_feedback_page";

export type FeedbackKind = "feedback" | "bug";

/** Opens the new-issue form in the browser. Rejects with Rust's message. */
export async function openFeedbackPage(kind: FeedbackKind): Promise<void> {
  try {
    await invoke(OPEN_FEEDBACK_PAGE, { kind });
  } catch (e) {
    throw new Error(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
  }
}
