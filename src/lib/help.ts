/** Where each help button leads in the published docs. Kept in one place so a renamed heading
 *  is caught by the test beside it, not by a user landing at the top of the wrong page. */

export const DOCS_URL = "https://wieluk.github.io/gameyfin-app/docs/";

export const HELP = {
  firstStart: "getting-started#first-start",
  library: "library#library",
  downloads: "library#downloads",
  installed: "library#installed-games",
  gamesFolders: "library#games-folders",
  saves: "saves#the-saves-page",
  saveSync: "saves#how-it-works",
  saveLocation: "saves#where-saves-are-kept",
  moveSaves: "saves#move-saves",
  windowsGames: "playing#windows-games-on-linux",
  prefixes: "playing#prefixes",
  controllers: "playing#controllers",
  window: "app#window-and-tray",
  notifications: "app#notifications",
  updates: "app#updates",
  account: "app#account",
  troubleshooting: "app#troubleshooting",
} as const;

export type HelpTopic = keyof typeof HELP;

export function helpUrl(topic: HelpTopic): string {
  return DOCS_URL + HELP[topic];
}
