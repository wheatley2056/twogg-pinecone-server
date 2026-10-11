import {
  BooleanField,
  BooleanInput,
  Create,
  Datagrid,
  Edit,
  ImageField,
  ImageInput,
  List,
  NumberField,
  NumberInput,
  required,
  SearchInput,
  SelectField,
  SelectInput,
  SimpleForm,
  TextField,
  TextInput,
} from "react-admin";

// TwoGG-Auswahl in „Entdecken“: kuratierte Projekte je Inhaltsart.
const TYPES = [
  { id: "modpack", name: "Modpacks" },
  { id: "mod", name: "Mods" },
  { id: "resourcepack", name: "Ressourcenpakete" },
  { id: "shader", name: "Shader" },
  { id: "datapack", name: "Datenpakete" },
];
const SOURCES = [
  { id: "twogg", name: "TwoGG-Pack" },
  { id: "modrinth", name: "Modrinth" },
  { id: "curseforge", name: "CurseForge" },
];
const MOTIFS = [
  { id: "shield", name: "Wappen" },
  { id: "block", name: "Block" },
  { id: "house", name: "Haus" },
  { id: "sliders", name: "Regler" },
  { id: "waves", name: "Wellen" },
  { id: "snowflake", name: "Schneeflocke" },
  { id: "server", name: "Server" },
  { id: "bolt", name: "Blitz" },
];

const filters = [
  <SearchInput key="q" source="q" alwaysOn />,
  <SelectInput key="t" source="projectType" choices={TYPES} label="Inhaltsart" />,
];

export const RecommendationList = () => (
  <List filters={filters} sort={{ field: "sortOrder", order: "ASC" }}>
    <Datagrid rowClick="edit" bulkActionButtons={false}>
      <ImageField source="iconUrl" sx={{ "& img": { width: 36, height: 36, objectFit: "cover" } }} sortable={false} />
      <TextField source="title" label="Titel" />
      <SelectField source="projectType" choices={TYPES} label="Inhaltsart" />
      <SelectField source="source" choices={SOURCES} label="Quelle" />
      <TextField source="projectId" label="Projekt" />
      <NumberField source="sortOrder" label="Reihenfolge" />
      <BooleanField source="visible" label="Sichtbar" />
    </Datagrid>
  </List>
);

const Fields = () => (
  <>
    <TextInput source="title" label="Titel" validate={required()} fullWidth />
    <TextInput source="subtitle" label="Untertitel" fullWidth helperText="Ein kurzer Satz, z. B. „Offizieller Look für REVIVAL“" />
    <SelectInput source="projectType" label="Inhaltsart" choices={TYPES} validate={required()} defaultValue="modpack" />
    <SelectInput source="source" label="Quelle" choices={SOURCES} validate={required()} defaultValue="modrinth" />
    <TextInput
      source="projectId"
      label="Projekt"
      validate={required()}
      helperText="Modrinth: ID oder Slug (z. B. fabulously-optimized) · CurseForge: Projektnummer · TwoGG: Pack-ID"
    />
    <SelectInput source="motif" label="Motiv (ohne Bild)" choices={MOTIFS} emptyText="—" />
    <ImageInput
      source="icon"
      label="Bild"
      accept={{ "image/*": [".png", ".jpg", ".jpeg", ".webp", ".gif"] }}
      format={(v: unknown) => (typeof v === "string" ? { src: `/media/${v}`, title: v } : v)}
    >
      <ImageField source="src" title="title" />
    </ImageInput>
    <NumberInput source="sortOrder" label="Reihenfolge" helperText="Kleinere Zahl = weiter vorn; der Launcher zeigt die ersten drei" />
    <BooleanInput source="visible" label="Sichtbar" defaultValue={true} />
  </>
);

export const RecommendationCreate = () => (
  <Create redirect="list">
    <SimpleForm>
      <Fields />
    </SimpleForm>
  </Create>
);

export const RecommendationEdit = () => (
  <Edit mutationMode="pessimistic">
    <SimpleForm>
      <Fields />
    </SimpleForm>
  </Edit>
);
