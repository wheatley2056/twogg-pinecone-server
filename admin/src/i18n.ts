import polyglotI18nProvider from "ra-i18n-polyglot";
import englishMessages from "ra-language-english";
// Ältere, aber weitgehend passende Übersetzung; fehlende Texte fallen auf Englisch zurück.
import germanMessages from "ra-language-german";
import { mergeTranslations } from "react-admin";

const de = mergeTranslations(englishMessages, germanMessages, {
  resources: {
    accounts: {
      name: "Konto |||| Konten",
      fields: { id: "Benutzername", role: "Rolle", lastLogin: "Zuletzt angemeldet", createdAt: "Angelegt", password: "Passwort" },
    },
    packs: {
      name: "Modpack |||| Modpacks",
      fields: {
        id: "ID",
        title: "Titel",
        summary: "Kurzbeschreibung",
        description: "Beschreibung (Markdown)",
        icon: "Icon",
        iconUrl: "Icon",
        visibility: "Sichtbarkeit",
        featured: "Hervorgehoben",
        sortOrder: "Reihenfolge",
        versionCount: "Versionen",
        downloads: "Downloads",
        updatedAt: "Geändert",
        createdAt: "Angelegt",
      },
    },
    versions: {
      name: "Version |||| Versionen",
      fields: {
        packId: "Modpack",
        version: "Version",
        minecraft: "Minecraft",
        loader: "Loader",
        loaderVersion: "Loader-Version",
        channel: "Kanal",
        changelog: "Änderungen (Markdown)",
        size: "Größe",
        downloads: "Downloads",
        publishedAt: "Veröffentlicht",
        file: "Modpack-Datei (.mrpack)",
      },
    },
    news: {
      name: "Neuigkeit |||| Neuigkeiten",
      fields: {
        title: "Titel",
        body: "Text (Markdown)",
        image: "Bild",
        imageUrl: "Bild",
        link: "Link",
        pinned: "Angepinnt",
        visible: "Sichtbar",
        publishedAt: "Veröffentlichung",
      },
    },
  },
});

export const i18nProvider = polyglotI18nProvider(
  (locale) => (locale === "en" ? englishMessages : de),
  "de",
  [
    { locale: "de", name: "Deutsch" },
    { locale: "en", name: "English" },
  ],
  { allowMissing: true },
);
