import { useEffect, useState } from "react";
import { Card, CardContent, Grid, Typography } from "@mui/material";
import { Title, useDataProvider } from "react-admin";
import type { Stats } from "./dataProvider";

const Tile = ({ label, value }: { label: string; value: number | string }) => (
  <Card>
    <CardContent>
      <Typography color="text.secondary" variant="body2">
        {label}
      </Typography>
      <Typography variant="h4">{value}</Typography>
    </CardContent>
  </Card>
);

export const Dashboard = () => {
  const dataProvider = useDataProvider();
  const [stats, setStats] = useState<Stats | null>(null);
  useEffect(() => {
    dataProvider.getStats().then(setStats).catch(() => setStats(null));
  }, [dataProvider]);
  return (
    <>
      <Title title="TwoGG Pinecone" />
      <Grid container spacing={2} sx={{ mt: 1 }}>
        <Grid size={{ xs: 12, sm: 6, md: 3 }}>
          <Tile label="Modpacks (öffentlich / gesamt)" value={stats ? `${stats.publicPacks} / ${stats.packs}` : "…"} />
        </Grid>
        <Grid size={{ xs: 12, sm: 6, md: 3 }}>
          <Tile label="Versionen" value={stats?.versions ?? "…"} />
        </Grid>
        <Grid size={{ xs: 12, sm: 6, md: 3 }}>
          <Tile label="Downloads" value={stats?.downloads ?? "…"} />
        </Grid>
        <Grid size={{ xs: 12, sm: 6, md: 3 }}>
          <Tile label="Neuigkeiten" value={stats?.news ?? "…"} />
        </Grid>
        <Grid size={12}>
          <Card>
            <CardContent>
              <Typography variant="h6">So kommt ein Modpack in den Launcher</Typography>
              <Typography component="ol" sx={{ pl: 2 }}>
                <li>In Pinecone das Profil zusammenstellen und unter „Teilen → Als Datei“ als .mrpack exportieren.</li>
                <li>Hier unter „Modpacks“ ein Pack anlegen (Titel, ID, Beschreibung, Icon).</li>
                <li>Im Pack „Neue Version hochladen“ und die .mrpack-Datei wählen.</li>
                <li>Sichtbarkeit auf „Öffentlich“ stellen. Pinecone zeigt es unter „Entdecken → TwoGG“.</li>
              </Typography>
            </CardContent>
          </Card>
        </Grid>
      </Grid>
    </>
  );
};
