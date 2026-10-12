import { Edit, SimpleForm, TextInput } from "react-admin";
import { Typography } from "@mui/material";
import { Navigate } from "react-router";

// Einstellungen sind ein einziger Datensatz `settings/main`; der Menüpunkt springt direkt dorthin.
export const SettingsRedirect = () => <Navigate to="/settings/main" replace />;

const https = (value: string) =>
  value && !/^https:\/\/\S+$/.test(value) ? "Bitte eine https-Adresse angeben" : undefined;

const email = (value: string) =>
  value && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value) ? "Ungültige E-Mail-Adresse" : undefined;

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
    </SimpleForm>
  </Edit>
);
