export default defineNuxtConfig({
  modules: ["@nuxt/ui"],
  css: ["~/assets/css/main.css"],
  ssr: false,
  app: {
    baseURL: "./",
  },
  nitro: {
    preset: "static",
  },
  devtools: { enabled: false },
  compatibilityDate: "2025-01-01",
});
