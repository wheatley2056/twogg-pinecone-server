import simpleRestProvider from "ra-data-simple-rest";
import { fetchUtils, HttpError, type DataProvider } from "react-admin";
import { getToken } from "./authProvider";

const API = "/api/admin";

const httpClient = (url: string, options: fetchUtils.Options = {}) => {
  const headers = new Headers(options.headers ?? { Accept: "application/json" });
  const token = getToken();
  if (token) headers.set("Authorization", `Bearer ${token}`);
  return fetchUtils.fetchJson(url, { ...options, headers }).catch((e: HttpError) => {
    // Fehlermeldung des Servers ({"error": "…"}) statt des HTTP-Statustexts anzeigen.
    const message = (e.body as { error?: string } | undefined)?.error ?? e.message;
    throw new HttpError(message, e.status, e.body);
  });
};

type Upload = { rawFile?: File; src?: string; title?: string } | string | null | undefined;

/** Bilder (Pack-Icon, Bild einer Neuigkeit) erst hochladen, dann nur den Dateinamen speichern. */
async function uploadImage(value: Upload): Promise<string | null | undefined> {
  if (value && typeof value === "object" && value.rawFile instanceof File) {
    const form = new FormData();
    form.append("file", value.rawFile);
    const { json } = await httpClient(`${API}/media`, { method: "POST", body: form });
    return json.name as string;
  }
  if (value && typeof value === "object") return value.title ?? null;
  return value;
}

const base = simpleRestProvider(API, httpClient);

async function prepare(resource: string, data: Record<string, unknown>) {
  const out = { ...data };
  if (resource === "packs" && "icon" in out) out.icon = await uploadImage(out.icon as Upload);
  if (resource === "news" && "image" in out) out.image = await uploadImage(out.image as Upload);
  if (resource === "recommendations" && "icon" in out) out.icon = await uploadImage(out.icon as Upload);
  return out;
}

export const dataProvider: DataProvider = {
  ...base,
  async create(resource, params) {
    if (resource === "versions") {
      // Modpack-Version: Datei als Multipart, Metadaten liest der Server aus dem .mrpack.
      const { file, ...fields } = params.data as { file?: { rawFile?: File } } & Record<string, unknown>;
      const form = new FormData();
      for (const [k, v] of Object.entries(fields)) if (v != null && v !== "") form.append(k, String(v));
      if (file?.rawFile) form.append("file", file.rawFile);
      const { json } = await httpClient(`${API}/versions`, { method: "POST", body: form });
      return { data: json };
    }
    return base.create(resource, { ...params, data: await prepare(resource, params.data) });
  },
  async update(resource, params) {
    return base.update(resource, { ...params, data: await prepare(resource, params.data) });
  },
  async getStats() {
    const { json } = await httpClient(`${API}/stats`);
    return json as Stats;
  },
};

export interface Stats {
  packs: number;
  publicPacks: number;
  versions: number;
  downloads: number;
  news: number;
}
