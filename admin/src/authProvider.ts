import type { AuthProvider } from "react-admin";

const TOKEN = "pinecone.admin.token";

export const getToken = () => localStorage.getItem(TOKEN);

/** Anmeldung gegen /api/admin/login; das Token gilt 12 Stunden (bzw. bis zum Server-Neustart). */
export const authProvider: AuthProvider = {
  async login({ username, password }) {
    const res = await fetch("/api/admin/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username, password }),
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.error ?? "Anmeldung fehlgeschlagen");
    localStorage.setItem(TOKEN, body.token);
  },
  async logout() {
    const token = getToken();
    localStorage.removeItem(TOKEN);
    if (token) await fetch("/api/admin/logout", { method: "POST", headers: { Authorization: `Bearer ${token}` } }).catch(() => {});
  },
  async checkAuth() {
    if (!getToken()) throw new Error("Bitte anmelden");
  },
  async checkError(error) {
    if (error?.status === 401) {
      localStorage.removeItem(TOKEN);
      throw new Error("Sitzung abgelaufen");
    }
  },
  async getIdentity() {
    const res = await fetch("/api/admin/me", { headers: { Authorization: `Bearer ${getToken()}` } });
    if (!res.ok) throw new Error("Nicht angemeldet");
    return res.json();
  },
};
