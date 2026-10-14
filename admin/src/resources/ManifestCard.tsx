import { useState } from "react";
import { Alert, Box, Button, Card, CardContent, Stack, Typography } from "@mui/material";
import VerifiedIcon from "@mui/icons-material/Verified";
import UploadFileIcon from "@mui/icons-material/UploadFile";
import { useDataProvider, useNotify, usePermissions, useRecordContext, useRefresh } from "react-admin";

/** Signiertes Event-Pack-Manifest einer Version (F-10): Status, hochladen, entfernen. Nur Admins. */
export const ManifestCard = () => {
  const r = useRecordContext<{ id: string; version: string; manifestKeyId?: string | null }>();
  const { permissions } = usePermissions();
  const dataProvider = useDataProvider();
  const notify = useNotify();
  const refresh = useRefresh();
  const [manifest, setManifest] = useState<File | null>(null);
  const [signature, setSignature] = useState<File | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!r || permissions === "editor") return null;

  const upload = async () => {
    if (!manifest || !signature) return;
    setBusy(true);
    setError(null);
    try {
      let parsed: unknown;
      try {
        parsed = JSON.parse(await manifest.text());
      } catch {
        throw new Error("Die Manifest-Datei ist kein JSON");
      }
      await dataProvider.setManifest(r.id, { manifest: parsed, signature: (await signature.text()).trim() });
      notify("Manifest geprüft und gespeichert", { type: "success" });
      setManifest(null);
      setSignature(null);
      refresh();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    try {
      await dataProvider.setManifest(r.id, null);
      notify("Manifest entfernt", { type: "info" });
      refresh();
    } catch (e) {
      notify((e as Error).message, { type: "error" });
    } finally {
      setBusy(false);
    }
  };

  const pick = (accept: string, file: File | null, set: (f: File | null) => void, label: string) => (
    <Button component="label" size="small" variant="outlined" startIcon={<UploadFileIcon />} sx={{ textTransform: "none" }}>
      {file ? file.name : label}
      <input hidden type="file" accept={accept} onChange={(e) => set(e.target.files?.[0] ?? null)} />
    </Button>
  );

  return (
    <Card variant="outlined" sx={{ mb: 2, width: "100%" }}>
      <CardContent>
        <Stack direction="row" spacing={2} alignItems="flex-start">
          <VerifiedIcon color={r.manifestKeyId ? "success" : "action"} sx={{ mt: 0.5 }} />
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <Typography variant="subtitle1">Event-Manifest</Typography>
            {r.manifestKeyId ? (
              <Typography variant="body2" color="text.secondary">
                Signiert mit Schlüssel <b>{r.manifestKeyId}</b>. Pinecone installiert diese Version als Event-Pack, prüft die
                Signatur und hält die Pflicht-Mods unverändert.
              </Typography>
            ) : (
              <>
                <Typography variant="body2" color="text.secondary">
                  Für Event-Packs: <code>pinecone eventpack sign</code> erzeugt zum .mrpack ein Manifest und eine Signatur.
                  Beides hier hochladen; der Server prüft, ob es zu dieser Version passt.
                </Typography>
                <Stack direction="row" spacing={1} sx={{ mt: 1.5, flexWrap: "wrap", rowGap: 1 }}>
                  {pick(".json,application/json", manifest, setManifest, "manifest.json wählen")}
                  {pick(".sig,text/plain", signature, setSignature, "Signatur (.sig) wählen")}
                </Stack>
              </>
            )}
            {error && (
              <Alert severity="error" sx={{ mt: 1.5 }}>
                {error}
              </Alert>
            )}
          </Box>
          <Box sx={{ flex: "none" }}>
            {r.manifestKeyId ? (
              <Button size="small" color="error" onClick={remove} disabled={busy}>
                Entfernen
              </Button>
            ) : (
              <Button size="small" variant="contained" onClick={upload} disabled={busy || !manifest || !signature}>
                Prüfen und speichern
              </Button>
            )}
          </Box>
        </Stack>
      </CardContent>
    </Card>
  );
};
