import {
  BooleanField,
  BooleanInput,
  Create,
  Datagrid,
  DateField,
  DateTimeInput,
  Edit,
  ImageField,
  ImageInput,
  List,
  required,
  SearchInput,
  SimpleForm,
  TextField,
  TextInput,
} from "react-admin";
import { fromInputDate, fromUnix, toInputDate } from "../format";

export const NewsList = () => (
  <List filters={[<SearchInput key="q" source="q" alwaysOn />]} sort={{ field: "publishedAt", order: "DESC" }}>
    <Datagrid rowClick="edit" bulkActionButtons={false}>
      <ImageField source="imageUrl" sx={{ "& img": { width: 64, height: 36, objectFit: "cover", borderRadius: 1 } }} sortable={false} />
      <TextField source="title" />
      <BooleanField source="pinned" />
      <BooleanField source="visible" />
      <DateField source="publishedAt" transform={fromUnix} showTime />
    </Datagrid>
  </List>
);

const NewsFields = () => (
  <>
    <TextInput source="title" validate={required()} fullWidth />
    <TextInput source="body" multiline minRows={8} fullWidth helperText="Markdown: **fett**, Listen, Links …" />
    <ImageInput
      source="image"
      accept={{ "image/*": [".png", ".jpg", ".jpeg", ".webp", ".gif"] }}
      format={(v: unknown) => (typeof v === "string" ? { src: `/media/${v}`, title: v } : v)}
    >
      <ImageField source="src" title="title" />
    </ImageInput>
    <TextInput source="link" fullWidth helperText="Optional: „Mehr lesen“-Link (https://…)" />
    <DateTimeInput source="publishedAt" format={toInputDate} parse={fromInputDate} helperText="In der Zukunft = erscheint erst dann" />
    <BooleanInput source="pinned" helperText="Bleibt oben" />
    <BooleanInput source="visible" defaultValue />
  </>
);

export const NewsCreate = () => (
  <Create redirect="list">
    <SimpleForm defaultValues={{ visible: true, publishedAt: Math.floor(Date.now() / 1000) }}>
      <NewsFields />
    </SimpleForm>
  </Create>
);

export const NewsEdit = () => (
  <Edit>
    <SimpleForm>
      <NewsFields />
    </SimpleForm>
  </Edit>
);
