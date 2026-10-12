import { DateTimeInput, Edit, SelectInput, SimpleForm, TextInput } from "react-admin";
import { fromInputDate, toInputDate } from "../format";
import { Typography } from "@mui/material";
import { Navigate } from "react-router";

// Einstellungen sind ein einziger Datensatz `settings/main`; der Menüpunkt springt direkt dorthin.
export const SettingsRedirect = () => <Navigate to="/settings/main" replace />;

const https = (value: string) =>
  value && !/^https:\/\/\S+$/.test(value) ? "Bitte eine https-Adresse angeben" : undefined;

const email = (value: string) =>
  value && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value) ? "Ungültige E-Mail-Adresse" : undefined;

const maxText = (value: string) => (value && value.length > 300 ? "Höchstens 300 Zeichen" : undefined);

const version = (value: string) =>
  value && !/^\d+\.\d+\.\d+$/.test(value) ? "Format 0.9.8 (nur Ziffern und Punkte)" : undefined;

const address = (value: string) =>
  value && !/^[A-Za-z0-9._:-]+$/.test(value) ? "Nur Host und optional :Port, z. B. mc.twogg.de" : undefined;

export const SettingsEdit = () => (
  <Edit title="Einstellungen" mutationMode="pessimistic" redirect={false}>
    <SimpleForm>
      <TextInput
        source="gameServerName"
        label="Name des Community-Servers"
        helperText="Erscheint im Launcher auf der Startseite, z. B. „TwoGG Server“"
        fullWidth
      />
      <TextInput
        source="gameServerAddress"
        label="Adresse für den Schnellbeitritt"
        helperText="Leer lassen, um die Kachel im Launcher auszublenden"
        validate={address}
        fullWidth
      />
      <Typography variant="subtitle2" sx={{ mt: 2 }}>
        Rechtliches
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
        Erscheint auf den Modpack-Seiten und im Launcher unter „Über Pinecone“. Die
        Minecraft-Nutzungsrichtlinien verlangen einen Kontakt, der kein Chat- oder Forenlink ist.
      </Typography>
      <TextInput source="imprintUrl" label="Impressum (https-Adresse)" validate={https} fullWidth />
      <TextInput source="privacyUrl" label="Datenschutzerklärung (https-Adresse)" validate={https} fullWidth />
      <TextInput source="contactEmail" label="Kontakt-E-Mail" validate={email} fullWidth />

      <Typography variant="subtitle2" sx={{ mt: 2 }}>
        Hinweis an alle Launcher
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
        Erscheint oben auf der Startseite, z. B. bei Wartung. Spieler können ihn schließen; ein geänderter Text erscheint
        wieder. Leer lassen, um keinen Hinweis zu zeigen.
      </Typography>
      <TextInput source="bannerText" label="Text" multiline fullWidth validate={maxText} helperText="Höchstens 300 Zeichen" />
      <SelectInput
        source="bannerLevel"
        label="Art"
        choices={[
          { id: "info", name: "Info" },
          { id: "warning", name: "Warnung" },
        ]}
        defaultValue="info"
      />
      <TextInput source="bannerLink" label="Link (optional, https)" validate={https} fullWidth />
      <DateTimeInput
        source="bannerUntil"
        label="Anzeigen bis (optional)"
        format={(v: unknown) => (v ? toInputDate(v) : "")}
        parse={(v: unknown) => fromInputDate(v) ?? 0}
        helperText="Danach verschwindet der Hinweis von selbst"
      />

      <Typography variant="subtitle2" sx={{ mt: 2 }}>
        Mindestversion des Launchers
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
        Ältere Launcher zeigen „Update nötig“. Bibliothek und Einzelspieler funktionieren weiter, TwoGG-Packs und Events
        erst nach dem Update.
      </Typography>
      <TextInput source="minLauncherVersion" label="Mindestversion (z. B. 0.9.8)" validate={version} />
    </SimpleForm>
  </Edit>
);
