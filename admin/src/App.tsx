import { Admin, Resource, defaultDarkTheme, defaultLightTheme } from "react-admin";
import ExtensionIcon from "@mui/icons-material/Extension";
import NewspaperIcon from "@mui/icons-material/Newspaper";
import LayersIcon from "@mui/icons-material/Layers";
import SettingsIcon from "@mui/icons-material/Settings";
import { authProvider } from "./authProvider";
import { dataProvider } from "./dataProvider";
import { i18nProvider } from "./i18n";
import { Dashboard } from "./Dashboard";
import { PackCreate, PackEdit, PackList } from "./resources/packs";
import { VersionCreate, VersionEdit, VersionList } from "./resources/versions";
import { NewsCreate, NewsEdit, NewsList } from "./resources/news";
import { SettingsEdit, SettingsRedirect } from "./resources/settings";

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
      <Resource name="packs" list={PackList} create={PackCreate} edit={PackEdit} icon={ExtensionIcon} recordRepresentation="title" />
      <Resource name="versions" list={VersionList} create={VersionCreate} edit={VersionEdit} icon={LayersIcon} recordRepresentation="version" />
      <Resource name="news" list={NewsList} create={NewsCreate} edit={NewsEdit} icon={NewspaperIcon} recordRepresentation="title" />
      <Resource name="settings" list={SettingsRedirect} edit={SettingsEdit} icon={SettingsIcon} options={{ label: "Einstellungen" }} />
    </Admin>
  );
}
