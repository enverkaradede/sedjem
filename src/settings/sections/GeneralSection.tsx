import { CornerDownLeft, Monitor, Power } from "lucide-react";

import { api } from "../../lib/ipc";
import type { InsertMethod, Settings } from "../../lib/types";
import { Button, Card, Row, SectionHeader, Segmented, Toggle } from "../ui";
import type { UpdateSettings } from "../useSettings";

const INSERT_OPTIONS = [
  { value: "paste", label: "Yapıştır" },
  { value: "type", label: "Doğrudan yaz" },
] as const satisfies ReadonlyArray<{ value: InsertMethod; label: string }>;

const INSERT_HELP: Record<InsertMethod, string> = {
  paste: "Pano + ⌘V: her uygulamada çalışır, uzun metin anında gelir; eski pano içeriği geri konur.",
  type: "Pano hiç kullanılmaz, metin tuş basışı olarak yazılır. Kod editörlerinde otomatik parantez/girinti tetiklenebilir.",
};

export function GeneralSection({ settings, update }: { settings: Settings; update: UpdateSettings }) {
  const g = settings.general;
  return (
    <>
      <SectionHeader title="Genel" subtitle="Sedjem'in nasıl görüneceği ve arka planda nasıl çalışacağı." />
      <div className="space-y-4">
        <Card title="Görünüm ve arka plan" icon={Monitor}>
          <Row label="Yüzen mikrofonu göster" hint="Tüm pencerelerin ve tam ekran uygulamaların üstünde durur. Sürükleyerek taşıyabilirsiniz.">
            <Toggle label="Yüzen mikrofonu göster" checked={g.panelVisible} onChange={(v) => void update((d) => void (d.general.panelVisible = v))} />
          </Row>
          <Row label="Dock'ta göster" hint="Kapalıyken Sedjem yalnızca menü çubuğunda yaşar.">
            <Toggle label="Dock'ta göster" checked={g.showInDock} onChange={(v) => void update((d) => void (d.general.showInDock = v))} />
          </Row>
          <Row label="Oturum açılışında başlat" hint="Mac açıldığında Sedjem arka planda sessizce başlar.">
            <Toggle label="Oturum açılışında başlat" checked={g.launchAtLogin} onChange={(v) => void update((d) => void (d.general.launchAtLogin = v))} />
          </Row>
        </Card>

        <Card title="Yazma davranışı" icon={CornerDownLeft}>
          <Row label="Ekleme yöntemi" hint={INSERT_HELP[g.insertMethod]}>
            <Segmented
              value={g.insertMethod}
              options={INSERT_OPTIONS}
              onChange={(method) => void update((d) => void (d.general.insertMethod = method))}
            />
          </Row>
          <Row label="Otomatik Enter" hint="Metin eklendikten sonra Return tuşuna basar; sohbet ve terminal için ideal.">
            <Toggle label="Otomatik Enter" checked={g.autoEnter} onChange={(v) => void update((d) => void (d.general.autoEnter = v))} />
          </Row>
          {g.insertMethod === "paste" && (
            <Row label="Panoyu geri yükle" hint="Yapıştırmadan sonra panodaki eski içeriği geri koyar.">
              <Toggle
                label="Panoyu geri yükle"
                checked={g.restoreClipboard}
                onChange={(v) => void update((d) => void (d.general.restoreClipboard = v))}
              />
            </Row>
          )}
          <Row label="Son dikteleri hatırla" hint="Yalnızca bellekte tutulur, diske yazılmaz; uygulama kapanınca silinir.">
            <Toggle label="Son dikteleri hatırla" checked={g.keepHistory} onChange={(v) => void update((d) => void (d.general.keepHistory = v))} />
          </Row>
        </Card>

        <Card
          title="Uygulama"
          icon={Power}
          description="Pencereyi kapatmak Sedjem'i kapatmaz; menü çubuğundaki kulak simgesinden her zaman ulaşabilirsiniz."
          actions={
            <Button variant="danger" icon={Power} onClick={() => void api.quitApp()}>
              Sedjem'den çık
            </Button>
          }
        />
      </div>
    </>
  );
}
