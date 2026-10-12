import { useEffect, useState } from "react";
import {
  Box,
  Card,
  CardContent,
  LinearProgress,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
  useTheme,
} from "@mui/material";
import { useDataProvider } from "react-admin";
import type { DailyStats } from "./dataProvider";

const dayLabel = (day: number, opts: Intl.DateTimeFormatOptions = { day: "numeric", month: "short" }) =>
  new Date(day * 86_400_000).toLocaleDateString("de-DE", { ...opts, timeZone: "UTC" });

const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0);

/** Neueste Version zuerst. */
const byVersionDesc = (a: string, b: string) => {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pb[i] ?? 0) - (pa[i] ?? 0);
  return 0;
};

/** Balken je Tag, mit Achsenbeschriftung am Anfang, in der Mitte und am Ende. */
const Bars = ({ values, from, height = 140 }: { values: number[]; from: number; height?: number }) => {
  const theme = useTheme();
  const max = Math.max(1, ...values);
  const w = 100 / values.length;
  const gap = values.length > 60 ? 0.15 : 0.25;
  const ticks = [0, Math.floor((values.length - 1) / 2), values.length - 1];
  return (
    <Box>
      <Typography variant="caption" color="text.secondary" component="div" sx={{ textAlign: "right", mb: 0.5 }}>
        Höchstwert {max}
      </Typography>
      <Box sx={{ height }}>
        <svg width="100%" height={height} viewBox={`0 0 100 ${height}`} preserveAspectRatio="none" role="img">
          {[0.5, 1].map((f) => (
            <line key={f} x1={0} x2={100} y1={height - f * (height - 4)} y2={height - f * (height - 4)} stroke={theme.palette.divider} strokeWidth={0.5} vectorEffect="non-scaling-stroke" />
          ))}
          {values.map((v, i) => {
            const h = (v / max) * (height - 4);
            return (
              <rect key={i} x={i * w + (w * gap) / 2} y={height - h} width={w * (1 - gap)} height={Math.max(h, v > 0 ? 1 : 0)} fill={theme.palette.primary.main}>
                <title>{`${dayLabel(from + i, { weekday: "short", day: "numeric", month: "short" })}: ${v}`}</title>
              </rect>
            );
          })}
        </svg>
      </Box>
      <Stack direction="row" justifyContent="space-between" sx={{ mt: 0.5 }}>
        {ticks.map((i) => (
          <Typography key={i} variant="caption" color="text.secondary">
            {dayLabel(from + i)}
          </Typography>
        ))}
      </Stack>
    </Box>
  );
};

/** Kleiner Verlauf für eine Tabellenzeile. */
const Spark = ({ values }: { values: number[] }) => {
  const theme = useTheme();
  const max = Math.max(1, ...values);
  const w = 100 / values.length;
  return (
    <svg width={140} height={24} viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden>
      {values.map((v, i) => (
        <rect key={i} x={i * w} y={24 - (v / max) * 24} width={w * 0.8} height={(v / max) * 24} fill={theme.palette.primary.main} />
      ))}
    </svg>
  );
};

const Section = ({ title, hint, children }: { title: string; hint: string; children: React.ReactNode }) => (
  <Card sx={{ height: "100%" }}>
    <CardContent>
      <Typography variant="h6">{title}</Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        {hint}
      </Typography>
      {children}
    </CardContent>
  </Card>
);

const Empty = () => (
  <Typography variant="body2" color="text.secondary" sx={{ py: 3, textAlign: "center" }}>
    Noch keine Daten in diesem Zeitraum
  </Typography>
);

export const Usage = () => {
  const dataProvider = useDataProvider();
  const [days, setDays] = useState(30);
  const [stats, setStats] = useState<DailyStats | null>(null);
  useEffect(() => {
    dataProvider.getDailyStats(days).then(setStats).catch(() => setStats(null));
  }, [dataProvider, days]);

  const launcherTotal = stats ? sum(stats.launchers.map((l) => l.total)) : 0;
  return (
    <Stack spacing={2}>
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Box>
          <Typography variant="h5">Nutzung</Typography>
          <Typography variant="body2" color="text.secondary">
            Nur Tageszähler, keine Namen, UUIDs oder IP-Adressen. Zähler älter als 400 Tage werden gelöscht.
          </Typography>
        </Box>
        <ToggleButtonGroup size="small" exclusive value={days} onChange={(_, v) => v && setDays(v)}>
          <ToggleButton value={7}>7 Tage</ToggleButton>
          <ToggleButton value={30}>30 Tage</ToggleButton>
          <ToggleButton value={90}>90 Tage</ToggleButton>
        </ToggleButtonGroup>
      </Stack>
      {!stats ? (
        <LinearProgress />
      ) : (
        <Box sx={{ display: "grid", gap: 2, gridTemplateColumns: { xs: "1fr", md: "3fr 2fr" } }}>
          <Section
            title="Aktive Spieler pro Tag"
            hint={`Freunde-Profile mit Anmeldung oder Status an dem Tag · im Schnitt ${Math.round(sum(stats.active) / stats.days)} pro Tag`}
          >
            <Bars values={stats.active} from={stats.from} />
          </Section>
          <Section title="Launcher nach Version" hint="Je Gerät und Tag einmal gezählt">
            {stats.launchers.length === 0 ? (
              <Empty />
            ) : (
              <Stack spacing={1.5}>
                {[...stats.launchers].sort((a, b) => byVersionDesc(a.key, b.key)).slice(0, 6).map((l) => (
                  <Box key={l.key}>
                    <Stack direction="row" justifyContent="space-between">
                      <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
                        {l.key}
                      </Typography>
                      <Typography variant="body2" color="text.secondary">
                        {Math.round((l.total / launcherTotal) * 100)} %
                      </Typography>
                    </Stack>
                    <LinearProgress variant="determinate" value={(l.total / launcherTotal) * 100} sx={{ height: 8, borderRadius: 1 }} />
                  </Box>
                ))}
              </Stack>
            )}
          </Section>
          <Box sx={{ gridColumn: { md: "1 / -1" } }}>
            <Section title="Downloads je Pack-Version" hint="Abgeschlossene Downloads über den Launcher">
              {stats.downloads.length === 0 ? (
                <Empty />
              ) : (
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Pack</TableCell>
                      <TableCell>Version</TableCell>
                      <TableCell>Verlauf</TableCell>
                      <TableCell align="right">Downloads</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {stats.downloads.slice(0, 10).map((d) => {
                      const [pack, version] = d.key.split("/");
                      return (
                        <TableRow key={d.key}>
                          <TableCell>{pack}</TableCell>
                          <TableCell sx={{ fontFamily: "monospace" }}>{version}</TableCell>
                          <TableCell>
                            <Spark values={d.counts} />
                          </TableCell>
                          <TableCell align="right">{d.total}</TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              )}
            </Section>
          </Box>
        </Box>
      )}
    </Stack>
  );
};
