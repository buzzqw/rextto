import { test, expect } from "@playwright/test";

const nav = (page: import("@playwright/test").Page, label: string) =>
  page.locator("aside .nav-item").filter({ hasText: label }).first();

test("dashboard and configuration areas are usable", async ({ page }) => {
  await page.goto("/");

  await expect(page).toHaveTitle("Rextto");
  await expect(page.locator("h1").filter({ hasText: "Dashboard" })).toBeVisible();
  await expect(page.getByText("Azioni rapide")).toBeVisible();
  await page.getByRole("button", { name: "Carica risultati" }).click();
  await expect(page.getByRole("button", { name: "Aggiorna risultati" })).toBeVisible();

  await nav(page, "Blocklist").click();
  await expect(page.locator("h3").filter({ hasText: "Blocklist" })).toBeVisible();

  await nav(page, "Integrazioni").click();
  await expect(page.locator("h3").filter({ hasText: "Trakt" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Registra gestore magnet nel browser" })).toBeVisible();

  await nav(page, "Archivio").click();
  await expect(page.locator("h3").filter({ hasText: "Archivio torrent" })).toBeVisible();
  await expect(page.getByText(/Pagina 1 \/ /)).toBeVisible();

  await nav(page, "Configurazione").click();
  await page.getByRole("button", { name: "Traduzioni" }).click();
  await expect(page.getByRole("button", { name: "Esporta" })).toBeVisible();

  await nav(page, "Manutenzione").click();
  await expect(page.getByRole("button", { name: "Backup", exact: true })).toBeVisible();

  // "Grafici"/"Attività" non sono più pagine a sé: i dati live sono in Salute/Log.
  await nav(page, "Salute").click();
  await expect(page.locator("h3").filter({ hasText: "Runtime" })).toBeVisible();
  await nav(page, "Log").click();
  await expect(page.locator("h3").filter({ hasText: "Log daemon" })).toBeVisible();

  await nav(page, "Licenza").click();
  await expect(page.locator("h3").filter({ hasText: "Licenza" })).toBeVisible();
});
