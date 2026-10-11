import {
  BooleanField,
  BooleanInput,
  Create,
  Datagrid,
  DateField,
  Edit,
  EditButton,
  FunctionField,
  ImageField,
  ImageInput,
  List,
  NumberField,
  NumberInput,
  ReferenceManyField,
  required,
  SearchInput,
  SelectField,
  SelectInput,
  SimpleForm,
  TabbedForm,
  TextField,
  TextInput,
  TopToolbar,
  useRecordContext,
  Button,
  Link,
} from "react-admin";
import AddIcon from "@mui/icons-material/Add";
import { bytes, fromUnix } from "../format";

export const VISIBILITY = [
  { id: "public", name: "Öffentlich (im Launcher)" },
  { id: "unlisted", name: "Nicht gelistet (nur per Link)" },
  { id: "hidden", name: "Versteckt" },
];

const ID_PATTERN = (v: string) =>
  v && !/^[a-z0-9][a-z0-9-]{0,46}[a-z0-9]$/.test(v) ? "2–48 Zeichen: a–z, 0–9, Bindestriche" : undefined;

const filters = [<SearchInput key="q" source="q" alwaysOn />, <SelectInput key="v" source="visibility" choices={VISIBILITY} />];

export const PackList = () => (
  <List filters={filters} sort={{ field: "updatedAt", order: "DESC" }}>
    <Datagrid rowClick="edit" bulkActionButtons={false}>
      <ImageField source="iconUrl" sx={{ "& img": { width: 36, height: 36, borderRadius: 1, objectFit: "cover" } }} sortable={false} />
      <TextField source="title" />
      <TextField source="id" />
      <SelectField source="visibility" choices={VISIBILITY} />
      <BooleanField source="featured" />
      <NumberField source="versionCount" sortable={false} />
      <NumberField source="downloads" sortable={false} />
      <DateField source="updatedAt" transform={fromUnix} showTime />
    </Datagrid>
  </List>
);

/** Icon-Feld: vorhandenes Bild anzeigen, neues hochladen. */
const IconInput = () => (
  <ImageInput
    source="icon"
    accept={{ "image/*": [".png", ".jpg", ".jpeg", ".webp", ".gif"] }}
    format={(v: unknown) => (typeof v === "string" ? { src: `/media/${v}`, title: v } : v)}
  >
    <ImageField source="src" title="title" />
  </ImageInput>
);

const PackFields = ({ create = false }: { create?: boolean }) => (
  <>
    <TextInput source="title" validate={required()} fullWidth />
    {create ? (
      <TextInput source="id" validate={[required(), ID_PATTERN]} helperText="Erscheint in Links, später nicht änderbar (z. B. horizons-unbound)" />
    ) : null}
    <TextInput source="summary" fullWidth helperText="Ein Satz für die Übersicht im Launcher" />
    <TextInput source="description" multiline minRows={6} fullWidth helperText="Markdown: **fett**, Listen, Links …" />
    <IconInput />
    <SelectInput source="visibility" choices={VISIBILITY} defaultValue="hidden" validate={required()} />
    <BooleanInput source="featured" helperText="Oben im Launcher zeigen" />
    <NumberInput source="sortOrder" helperText="Kleinere Zahl = weiter oben" />
  </>
);

export const PackCreate = () => (
  <Create redirect="edit">
    <SimpleForm>
      <PackFields create />
    </SimpleForm>
  </Create>
);

const NewVersionButton = () => {
  const record = useRecordContext();
  if (!record) return null;
  return (
    <Button
      component={Link}
      to={{ pathname: "/versions/create", search: `?source=${encodeURIComponent(JSON.stringify({ packId: record.id }))}` }}
      label="Neue Version hochladen"
    >
      <AddIcon />
    </Button>
  );
};

export const PackEdit = () => (
  <Edit actions={<TopToolbar><NewVersionButton /></TopToolbar>}>
    <TabbedForm>
      <TabbedForm.Tab label="Allgemein">
        <TextField source="id" />
        <PackFields />
      </TabbedForm.Tab>
      <TabbedForm.Tab label="Versionen">
        <NewVersionButton />
        <ReferenceManyField reference="versions" target="packId" sort={{ field: "publishedAt", order: "DESC" }}>
          <Datagrid rowClick="edit" bulkActionButtons={false} empty={<p>Noch keine Version. Exportiere das Pack in Pinecone als .mrpack und lade es hier hoch.</p>}>
            <TextField source="version" />
            <TextField source="minecraft" />
            <TextField source="loader" />
            <TextField source="channel" />
            <FunctionField source="size" render={(r: { size: number }) => bytes(r.size)} />
            <NumberField source="downloads" />
            <DateField source="publishedAt" transform={fromUnix} showTime />
            <EditButton />
          </Datagrid>
        </ReferenceManyField>
      </TabbedForm.Tab>
    </TabbedForm>
  </Edit>
);
