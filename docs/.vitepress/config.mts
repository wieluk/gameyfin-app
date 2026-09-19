import { defineConfig } from "vitepress";

// Published to gh-pages/docs/, beside the Flatpak repository in gh-pages/repo/.
export default defineConfig({
  title: "gameyfin-app",
  description: "Documentation for gameyfin-app, a Gameyfin client for Windows and Linux.",
  base: "/gameyfin-app/docs/",
  // The docs index is the README GitHub shows for the folder.
  rewrites: { "README.md": "index.md" },
  cleanUrls: true,
  themeConfig: {
    sidebar: [
      { text: "Getting started", link: "/getting-started" },
      { text: "Library, downloads and installing", link: "/library" },
      { text: "Playing games", link: "/playing" },
      { text: "Save sync", link: "/saves" },
      { text: "App settings", link: "/app" },
    ],
    search: { provider: "local" },
    socialLinks: [{ icon: "github", link: "https://github.com/wieluk/gameyfin-app" }],
  },
});
