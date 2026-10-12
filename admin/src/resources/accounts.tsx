import {
  Create,
  Datagrid,
  DateField,
  DeleteButton,
  Edit,
  FunctionField,
  List,
  PasswordInput,
  SaveButton,
  SelectField,
  SelectInput,
  SimpleForm,
  TextField,
  TextInput,
  Toolbar,
  required,
  useRecordContext,
} from "react-admin";
import { Chip, Typography } from "@mui/material";
import { fromUnix } from "../format";

// Konten der Admin-Oberfläche mit Rolle (F-63). Nur für Admins sichtbar.
const roles = [
  { id: "admin", name: "Admin – alles, auch Packs, Einstellungen und Konten" },
  { id: "editor", name: "Redaktion – Neuigkeiten, TwoGG-Auswahl, Bilder" },
];
const roleShort = [
  { id: "admin", name: "Admin" },
  { id: "editor", name: "Redaktion" },
];

const minLength = (v: string) => (v && v.length < 10 ? "Mindestens 10 Zeichen" : undefined);
const username = (v: string) =>
  v && !/^[A-Za-z0-9._-]{2,32}$/.test(v) ? "2–32 Zeichen: Buchstaben, Ziffern, . _ -" : undefined;

export const AccountList = () => (
  <List sort={{ field: "id", order: "ASC" }} exporter={false}>
    <Datagrid rowClick="edit" bulkActionButtons={false}>
      <FunctionField
        source="id"
        render={(r: { id: string; protected?: boolean }) => (
          <>
            {r.id}
            {r.protected && <Chip size="small" label="aus Konfiguration" sx={{ ml: 1 }} />}
          </>
        )}
      />
      <SelectField source="role" choices={roleShort} />
      <FunctionField
        source="lastLogin"
        render={(r: { lastLogin: number | null }) => (r.lastLogin ? fromUnix(r.lastLogin).toLocaleString("de-DE") : "noch nie")}
      />
      <DateField source="createdAt" transform={fromUnix} locales="de-DE" />
    </Datagrid>
  </List>
);

export const AccountCreate = () => (
  <Create redirect="list">
    <SimpleForm>
      <TextInput source="id" label="Benutzername" validate={[required(), username]} />
      <SelectInput source="role" choices={roles} defaultValue="editor" validate={required()} />
      <PasswordInput source="password" label="Passwort" validate={[required(), minLength]} />
    </SimpleForm>
  </Create>
);

const EditToolbar = () => {
  const record = useRecordContext<{ protected?: boolean }>();
  return (
    <Toolbar sx={{ justifyContent: "space-between" }}>
      <SaveButton />
      {!record?.protected && <DeleteButton mutationMode="pessimistic" />}
    </Toolbar>
  );
};

const ProtectedHint = () => {
  const record = useRecordContext<{ protected?: boolean }>();
  return record?.protected ? (
    <Typography variant="body2" color="text.secondary">
      Dieses Konto kommt aus der Server-Konfiguration (PINECONE_ADMIN_USER). Es bleibt Admin, und sein Passwort
      wird dort geändert.
    </Typography>
  ) : null;
};

const AccountTitle = () => {
  const record = useRecordContext<{ id: string }>();
  return <>Konto {record?.id}</>;
};

export const AccountEdit = () => (
  <Edit mutationMode="pessimistic" redirect="list" title={<AccountTitle />}>
    <SimpleForm toolbar={<EditToolbar />}>
      <TextField source="id" label="Benutzername" />
      <ProtectedHint />
      <SelectInput source="role" choices={roles} validate={required()} />
      <PasswordInput source="password" label="Neues Passwort" helperText="Leer lassen, um es nicht zu ändern" validate={minLength} />
    </SimpleForm>
  </Edit>
);
