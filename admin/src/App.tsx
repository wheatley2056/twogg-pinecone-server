import { Admin, Resource, defaultDarkTheme, defaultLightTheme } from "react-admin";
import ExtensionIcon from "@mui/icons-material/Extension";
import NewspaperIcon from "@mui/icons-material/Newspaper";
import LayersIcon from "@mui/icons-material/Layers";
import SettingsIcon from "@mui/icons-material/Settings";
import StarIcon from "@mui/icons-material/Star";
import PeopleIcon from "@mui/icons-material/People";
import HistoryIcon from "@mui/icons-material/History";
import { AccountCreate, AccountEdit, AccountList } from "./resources/accounts";
import { AuditList } from "./resources/audit";
import { authProvider } from "./authProvider";
import { dataProvider } from "./dataProvider";
import { i18nProvider } from "./i18n";
import { Dashboard } from "./Dashboard";
import { PackCreate, PackEdit, PackList } from "./resources/packs";
import { VersionCreate, VersionEdit, VersionList } from "./resources/versions";
import { NewsCreate, NewsEdit, NewsList } from "./resources/news";
import { SettingsEdit, SettingsRedirect } from "./resources/settings";
import { RecommendationCreate, RecommendationEdit, RecommendationList } from "./resources/recommendations";

// Harz aus dem Pinecone-Design-System als Primärfarbe.
const accent = { palette: { primary: { main: "#e9a23b" } } };
const lightTheme = { ...defaultLightTheme, palette: { ...defaultLightTheme.palette, ...accent.palette } };
const darkTheme = { ...defaultDarkTheme, palette: { ...defaultDarkTheme.palette, ...accent.palette } };

export default function App() {
  return (
    <Admin
      title="Pinecone Admin"
      dashboard={Dashboard}
      authProvider={authProvider}
      dataProvider={dataProvider}
      i18nProvider={i18nProvider}
      theme={lightTheme}
      darkTheme={darkTheme}
      defaultTheme="dark"
      requireAuth
    >
      {(role: string) => {
        // Redaktion: Packs und Versionen nur lesen, keine Einstellungen, Konten, Protokoll (F-63).
        const admin = role !== "editor";
        return (
          <>
            <Resource
              name="packs"
              list={PackList}
              create={admin ? PackCreate : undefined}
              edit={admin ? PackEdit : undefined}
              icon={ExtensionIcon}
              recordRepresentation="title"
            />
            <Resource
              name="versions"
              list={VersionList}
              create={admin ? VersionCreate : undefined}
              edit={admin ? VersionEdit : undefined}
              icon={LayersIcon}
              recordRepresentation="version"
            />
            <Resource name="news" list={NewsList} create={NewsCreate} edit={NewsEdit} icon={NewspaperIcon} recordRepresentation="title" />
            <Resource
              name="recommendations"
              list={RecommendationList}
              create={RecommendationCreate}
              edit={RecommendationEdit}
              icon={StarIcon}
              options={{ label: "TwoGG-Auswahl" }}
              recordRepresentation="title"
            />
            {admin && <Resource name="settings" list={SettingsRedirect} edit={SettingsEdit} icon={SettingsIcon} options={{ label: "Einstellungen" }} />}
            {admin && (
              <Resource
                name="accounts"
                list={AccountList}
                create={AccountCreate}
                edit={AccountEdit}
                icon={PeopleIcon}
                options={{ label: "Konten" }}
                recordRepresentation="id"
              />
            )}
            {admin && <Resource name="audit" list={AuditList} icon={HistoryIcon} options={{ label: "Protokoll" }} />}
          </>
        );
      }}
    </Admin>
  );
}
