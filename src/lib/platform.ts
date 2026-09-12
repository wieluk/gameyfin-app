/** Which desktop the webview is running on. Read from the user agent: the check is needed
 * before any command round trip, and Tauri reports the host here. */

/** Windows runs its own programs; none of the compatibility machinery applies there. */
export const isWindows =
  typeof navigator !== "undefined" && /win/i.test(navigator.platform || navigator.userAgent);
