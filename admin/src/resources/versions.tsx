import {
  Create,
  Datagrid,
  DateField,
  Edit,
  FileField,
  FileInput,
  FunctionField,
  List,
  NumberField,
  ReferenceField,
  ReferenceInput,
  required,
  SelectInput,
  SimpleForm,
  TextField,
  TextInput,
  AutocompleteInput,
} from "react-admin";
import { bytes, fromUnix } from "../format";

export const CHANNELS = [
  { id: "release", name: "Release" },
  { id: "beta", name: "Beta" },
  { id: "alpha", name: "Alpha" },
];

const filters = [
  <ReferenceInput key="p" source="packId" reference="packs" alwaysOn>
    <AutocompleteInput optionText="title" label="Modpack" />
  </ReferenceInput>,
  <SelectInput key="c" source="channel" choices={CHANNELS} />,
];

export const VersionList = () => (
  <List filters={filters} sort={{ field: "publishedAt", order: "DESC" }}>
    <Datagrid rowClick="edit" bulkActionButtons={false}>
      <ReferenceField source="packId" reference="packs" link="edit">
        <TextField source="title" />
      </ReferenceField>
      <TextField source="version" />
      <TextField source="minecraft" />
      <TextField source="loader" />
      <TextField source="channel" />
      <FunctionField source="size" render={(r: { size: number }) => bytes(r.size)} sortable={false} />
      <NumberField source="downloads" />
      <DateField source="publishedAt" transform={fromUnix} showTime />
    </Datagrid>
  </List>
);

export const VersionCreate = () => (
  <Create redirect={(_resource, _id, data) => `packs/${encodeURIComponent(String(data?.packId))}/1`}>
    <SimpleForm>
      <ReferenceInput source="packId" reference="packs">
        <AutocompleteInput optionText="title" validate={required()} />
      </ReferenceInput>
      <FileInput source="file" accept={{ "application/zip": [".mrpack"] }} validate={required()}
        helperText="In Pinecone: Profil → Teilen → Als Datei (.mrpack). Minecraft-Version und Loader liest der Server aus der Datei.">
        <FileField source="src" title="title" />
      </FileInput>
      <TextInput source="version" helperText="Leer lassen = Versionsnummer aus dem Pack" />
      <SelectInput source="channel" choices={CHANNELS} defaultValue="release" validate={required()} />
      <TextInput source="changelog" multiline minRows={4} fullWidth />
    </SimpleForm>
  </Create>
);

export const VersionEdit = () => (
  <Edit redirect={(_resource, _id, data) => `packs/${encodeURIComponent(String(data?.packId))}/1`}>
    <SimpleForm>
      <ReferenceField source="packId" reference="packs" link="edit">
        <TextField source="title" />
      </ReferenceField>
      <TextField source="minecraft" />
      <TextField source="loader" />
      <TextField source="loaderVersion" />
      <TextField source="sha1" label="SHA-1" />
      <TextInput source="version" validate={required()} />
      <SelectInput source="channel" choices={CHANNELS} validate={required()} />
      <TextInput source="changelog" multiline minRows={4} fullWidth />
    </SimpleForm>
  </Edit>
);
