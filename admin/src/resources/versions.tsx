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
  useDataProvider,
  useNotify,
  usePermissions,
  useRecordContext,
  useRefresh,
} from "react-admin";
import { useState } from "react";
import { Alert, Button, Chip, Dialog, DialogActions, DialogContent, DialogTitle, TextField as MuiTextField } from "@mui/material";
import BlockIcon from "@mui/icons-material/Block";
import RestoreIcon from "@mui/icons-material/Restore";
import { bytes, fromUnix } from "../format";

type VersionRecord = { id: string; packId: string; version: string; withdrawnReason?: string | null; withdrawnAt?: number | null };

/** Status-Spalte: „zurückgezogen“ mit Grund als Tooltip. */
const WithdrawnChip = () => {
  const r = useRecordContext<VersionRecord>();
  return r?.withdrawnAt ? <Chip size="small" color="error" variant="outlined" label="zurückgezogen" title={r.withdrawnReason ?? ""} /> : null;
};

/** Zurückziehen (mit Grund) bzw. wiederherstellen (F-62). */
const WithdrawControls = () => {
  const r = useRecordContext<VersionRecord>();
  const { permissions } = usePermissions();
  const dataProvider = useDataProvider();
  const notify = useNotify();
  const refresh = useRefresh();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  if (!r || permissions === "editor") return null;

  const call = async (action: "withdraw" | "restore") => {
    setBusy(true);
    try {
      await dataProvider.versionAction(r.id, action, action === "withdraw" ? { reason } : undefined);
      notify(action === "withdraw" ? "Version zurückgezogen" : "Version wieder verfügbar", { type: "success" });
      setOpen(false);
      setReason("");
      refresh();
    } catch (e) {
      notify((e as Error).message, { type: "error" });
    } finally {
      setBusy(false);
    }
  };

  if (r.withdrawnAt) {
    return (
      <Alert
        severity="warning"
        sx={{ mb: 2, width: "100%" }}
        action={
          <Button color="inherit" size="small" startIcon={<RestoreIcon />} disabled={busy} onClick={() => call("restore")}>
            Wiederherstellen
          </Button>
        }
      >
        Zurückgezogen am {fromUnix(r.withdrawnAt).toLocaleString("de-DE")}: {r.withdrawnReason}
      </Alert>
    );
  }
  return (
    <>
      <Button color="error" variant="outlined" startIcon={<BlockIcon />} onClick={() => setOpen(true)} sx={{ mb: 2 }}>
        Version zurückziehen
      </Button>
      <Dialog open={open} onClose={() => !busy && setOpen(false)} fullWidth maxWidth="sm">
        <DialogTitle>Version {r.version} zurückziehen?</DialogTitle>
        <DialogContent>
          <Alert severity="info" sx={{ mb: 2 }}>
            Die Version verschwindet aus dem Launcher, der Pack-Seite und den Updates. Wer sie installiert hat, sieht den
            Grund und kann auf die beste noch verfügbare Version wechseln, auch eine ältere. Datei und Downloads bleiben
            erhalten; du kannst sie jederzeit wiederherstellen.
          </Alert>
          <MuiTextField
            autoFocus
            fullWidth
            multiline
            minRows={2}
            label="Grund (sehen die Spieler)"
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            inputProps={{ maxLength: 300 }}
          />
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setOpen(false)} disabled={busy}>
            Abbrechen
          </Button>
          <Button color="error" variant="contained" disabled={busy || !reason.trim()} onClick={() => call("withdraw")}>
            Zurückziehen
          </Button>
        </DialogActions>
      </Dialog>
    </>
  );
};

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
      <FunctionField label="Status" render={() => <WithdrawnChip />} sortable={false} />
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
      <WithdrawControls />
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
