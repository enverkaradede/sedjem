import { Accessibility, Mic, ShieldCheck } from "lucide-react";
import { useState } from "react";

import { api } from "../../lib/ipc";
import { toAppError, type MicrophoneStatus, type Permissions } from "../../lib/types";
import { useToasts } from "../toasts";
import { Button, Card, SectionHeader, StatusDot } from "../ui";

const MIC_TEXT: Record<MicrophoneStatus, string> = {
  granted: "İzin verildi.",
  denied: "Reddedildi. Sistem Ayarları'ndan açmanız gerekiyor.",
  restricted: "Sistem politikası tarafından kısıtlanmış.",
  notDetermined: "Henüz sorulmadı.",
  unknown: "Durum okunamadı.",
};

export function PermissionsSection({
  permissions,
  onRefresh,
}: {
  permissions: Permissions | null;
  onRefresh: (next: Permissions) => void;
}) {
  const { push } = useToasts();
  const [asking, setAsking] = useState(false);

  const requestMic = () => {
    setAsking(true);
    api
      .requestMicrophone()
      .then(() => api.getPermissions())
      .then(onRefresh)
      .catch((err: unknown) => push("error", toAppError(err).message))
      .finally(() => setAsking(false));
  };

  const openPane = (pane: "accessibility" | "microphone") =>
    api.openPrivacySettings(pane).catch((err: unknown) => push("error", toAppError(err).message));

  const ax = permissions?.accessibility ?? false;
  const mic = permissions?.microphone ?? "unknown";

  return (
    <>
      <SectionHeader title="İzinler" subtitle="Sedjem iki macOS izniyle çalışır. Ses ve metin bu Mac'ten çıkmaz (ağ motorlarını seçmediğiniz sürece)." />
      <div className="space-y-4">
        <Card
          title="Erişilebilirlik"
          icon={Accessibility}
          description="Yazıya dökülen metni o an açık olan uygulamaya yapıştırmak (⌘V) ve isteğe bağlı Enter'a basmak için."
          actions={
            !ax && (
              <Button variant="primary" onClick={() => void openPane("accessibility")}>
                Sistem Ayarları'nı aç
              </Button>
            )
          }
        >
          <Status tone={ax ? "ok" : "warn"} text={ax ? "İzin verildi." : "İzin yok — listede Sedjem'i bulup açın."} />
          {!ax && (
            <p className="mt-3 text-[12px] leading-relaxed text-ink-400">
              Geliştirme sürümünde her yeniden derlemeden sonra macOS izni geçersiz sayabilir; bu durumda Sedjem'i listeden kaldırıp yeniden ekleyin.
            </p>
          )}
        </Card>

        <Card
          title="Mikrofon"
          icon={Mic}
          description="Konuşmanızı duymak için. Kayıtlar diske yazılmaz."
          actions={
            mic === "notDetermined" ? (
              <Button variant="primary" loading={asking} onClick={requestMic}>
                İzin iste
              </Button>
            ) : mic !== "granted" ? (
              <Button variant="primary" onClick={() => void openPane("microphone")}>
                Sistem Ayarları'nı aç
              </Button>
            ) : null
          }
        >
          <Status tone={mic === "granted" ? "ok" : mic === "notDetermined" ? "warn" : "error"} text={MIC_TEXT[mic]} />
        </Card>

        <div className="flex items-start gap-3 rounded-2xl border border-white/[0.06] bg-lapis-900/40 p-4 text-[12.5px] leading-relaxed text-ink-400">
          <ShieldCheck className="mt-0.5 size-4 shrink-0 text-jade" />
          Transkriptler hiçbir zaman loglanmaz; API anahtarları macOS Anahtar Zinciri'nde tutulur; pano içeriğiniz yapıştırmadan sonra geri yüklenir ve
          pano yöneticilerine “geçici” olarak işaretlenir.
        </div>
      </div>
    </>
  );
}

function Status({ tone, text }: { tone: "ok" | "warn" | "error"; text: string }) {
  return (
    <div className="flex items-center gap-2.5 text-[13px] text-ink-200">
      <StatusDot tone={tone} />
      {text}
    </div>
  );
}
