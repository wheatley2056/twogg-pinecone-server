import { Datagrid, DateField, FunctionField, List, SelectInput, TextField, TextInput } from "react-admin";
import { fromUnix } from "../format";

// Änderungsprotokoll (F-63): nur lesen, Einträge verschwinden nach einem Jahr.
const objects = [
  { id: "packs", name: "Modpacks" },
  { id: "versions", name: "Versionen" },
  { id: "news", name: "Neuigkeiten" },
  { id: "recommendations", name: "TwoGG-Auswahl" },
  { id: "settings", name: "Einstellungen" },
  { id: "accounts", name: "Konten" },
  { id: "media", name: "Bilder" },
  { id: "session", name: "Anmeldungen" },
];
const actions: Record<string, string> = {
  create: "angelegt",
  update: "geändert",
  delete: "gelöscht",
  login: "angemeldet",
};
const objectName = (id: string) => objects.find((o) => o.id === id)?.name ?? id;

export const AuditList = () => (
  <List
    sort={{ field: "at", order: "DESC" }}
    exporter={false}
    filters={[
      <TextInput key="u" source="username" label="Konto" alwaysOn />,
      <SelectInput key="o" source="objectType" label="Bereich" choices={objects} alwaysOn />,
    ]}
  >
    <Datagrid bulkActionButtons={false} rowClick={false}>
      <DateField source="at" transform={fromUnix} showTime locales="de-DE" label="Zeit" />
      <TextField source="username" label="Konto" />
      <FunctionField label="Aktion" render={(r: { action: string }) => actions[r.action] ?? r.action} />
      <FunctionField source="objectType" label="Bereich" render={(r: { objectType: string }) => objectName(r.objectType)} />
      <TextField source="objectId" label="Objekt" emptyText="–" />
    </Datagrid>
  </List>
);
