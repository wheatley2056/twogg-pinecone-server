import { Edit, SimpleForm, TextInput } from "react-admin";
import { Navigate } from "react-router";

// Einstellungen sind ein einziger Datensatz `settings/main`; der Menüpunkt springt direkt dorthin.
export const SettingsRedirect = () => <Navigate to="/settings/main" replace />;

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
    </SimpleForm>
  </Edit>
);
