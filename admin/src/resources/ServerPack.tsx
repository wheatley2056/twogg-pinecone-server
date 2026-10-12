import { useEffect, useState } from "react";
import { Alert, Box, Button, Card, CardContent, LinearProgress, Stack, Typography } from "@mui/material";
import DnsIcon from "@mui/icons-material/Dns";
import DownloadIcon from "@mui/icons-material/Download";
import RefreshIcon from "@mui/icons-material/Refresh";
import { useDataProvider, useNotify, usePermissions, useRecordContext } from "react-admin";
import type { ServerPack } from "../dataProvider";
import { bytes, fromUnix } from "../format";

/** Server-Pack aus dieser Version erzeugen und herunterladen (F-61), nur für Admins. */
export const ServerPackCard = () => {
  const r = useRecordContext<{ id: string; version: string }>();
  const { permissions } = usePermissions();
  const dataProvider = useDataProvider();
  const notify = useNotify();
  const [pack, setPack] = useState<ServerPack | null>(null);
  const id = r?.id;
  const admin = permissions !== "editor";

  useEffect(() => {
    if (!id || !admin) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const p: ServerPack = await dataProvider.serverPack(id);
        if (stop) return;
        setPack(p);
        if (p.state === "building") timer = setTimeout(poll, 2000);
      } catch {
        if (!stop) setPack(null);
      }
    };
    if (!pack || pack.state === "building") poll();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, admin, pack?.state]);

  if (!id || !admin) return null;

  const build = async () => {
    try {
      setPack(await dataProvider.serverPack(id, true));
    } catch (e) {
      notify((e as Error).message, { type: "error" });
    }
  };
  const download = async () => {
    try {
      window.location.href = await dataProvider.serverPackLink(id);
    } catch (e) {
      notify((e as Error).message, { type: "error" });
    }
  };

  const removed = pack?.removed ?? [];
  return (
    <Card variant="outlined" sx={{ mb: 2, width: "100%" }}>
      <CardContent>
        <Stack direction="row" spacing={2} alignItems="flex-start">
          <DnsIcon color="action" sx={{ mt: 0.5 }} />
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <Typography variant="subtitle1">Server-Pack</Typography>
            {!pack ? (
              <LinearProgress sx={{ mt: 1 }} />
            ) : pack.state === "building" ? (
              <>
                <Typography variant="body2" color="text.secondary">
                  Wird erzeugt … Mods werden geladen und geprüft. Das kann ein paar Minuten dauern.
                </Typography>
                <LinearProgress sx={{ mt: 1.5 }} />
              </>
            ) : pack.state === "ready" ? (
              <>
                <Typography variant="body2" color="text.secondary">
                  Erzeugt am {fromUnix(pack.createdAt).toLocaleString("de-DE")} · {bytes(pack.size ?? 0)} · {pack.files} Dateien
                  {pack.loader ? ` · ${pack.loader}` : ""}
                </Typography>
                {removed.length > 0 && (
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    Ohne {removed.length} reine Client-Mods: {removed.slice(0, 5).join(", ")}
                    {removed.length > 5 ? ` und ${removed.length - 5} weitere` : ""}
                  </Typography>
                )}
              </>
            ) : (
              <>
                <Typography variant="body2" color="text.secondary">
                  ZIP für den Minecraft-Server: ohne Mods, die nur der Client braucht, mit server-overrides und
                  Fabric-Starter bzw. Anleitung für den Loader-Installer.
                </Typography>
                {pack.state === "failed" && (
                  <Alert severity="error" sx={{ mt: 1.5 }}>
                    Erzeugen fehlgeschlagen: {pack.error}
                  </Alert>
                )}
              </>
            )}
          </Box>
          <Stack direction="row" spacing={1} sx={{ flex: "none" }}>
            {pack?.state === "ready" && (
              <>
                <Button size="small" startIcon={<RefreshIcon />} onClick={build}>
                  Neu erzeugen
                </Button>
                <Button size="small" variant="contained" startIcon={<DownloadIcon />} onClick={download}>
                  Herunterladen
                </Button>
              </>
            )}
            {(pack?.state === "none" || pack?.state === "failed") && (
              <Button size="small" variant="contained" startIcon={<DnsIcon />} onClick={build}>
                {pack.state === "failed" ? "Erneut versuchen" : "Server-Pack erzeugen"}
              </Button>
            )}
          </Stack>
        </Stack>
      </CardContent>
    </Card>
  );
};
