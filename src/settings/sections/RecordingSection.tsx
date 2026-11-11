import { AudioLines, Mic, Timer } from "lucide-react";
import { useEffect, useState } from "react";

import { api } from "../../lib/ipc";
import type { Settings } from "../../lib/types";
import { Card, Row, SectionHeader, Select, Slider, Toggle } from "../ui";
import type { UpdateSettings } from "../useSettings";

const SYSTEM_DEFAULT = "__system__";

function formatMs(ms: number): string {
  return `${ms} ms`;
}

function formatSecondsShort(ms: number): string {
  return `${(ms / 1000).toFixed(ms % 1000 === 0 ? 0 : 2).replace(".", ",")} sn`;
}

function formatDuration(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return minutes > 0 ? `${minutes} dk${rest > 0 ? ` ${rest} sn` : ""}` : `${seconds} sn`;
}

export function RecordingSection({ settings, update }: { settings: Settings; update: UpdateSettings }) {
  const r = settings.recording;
  const [devices, setDevices] = useState<string[]>([]);

  useEffect(() => {
    api.listInputDevices().then(setDevices).catch(() => setDevices([]));
  }, []);

  const deviceOptions = [
    { value: SYSTEM_DEFAULT, label: "Sistem varsayılanı" },
    ...devices.map((name) => ({ value: name, label: name })),
    ...(r.inputDevice && !devices.includes(r.inputDevice) ? [{ value: r.inputDevice, label: `${r.inputDevice} (bağlı değil)` }] : []),
  ];

  return (
    <>
      <SectionHeader title="Kayıt" subtitle="Mikrofon, zamanlama ve sessizlik davranışı." />
      <div className="space-y-4">
        <Card title="Mikrofon" icon={Mic}>
          <Row label="Giriş aygıtı">
            <Select
              label="Giriş aygıtı"
              className="max-w-[260px]"
              value={r.inputDevice ?? SYSTEM_DEFAULT}
              options={deviceOptions}
              onChange={(value) => void update((d) => void (d.recording.inputDevice = value === SYSTEM_DEFAULT ? null : value))}
            />
          </Row>
        </Card>

        <Card title="Zamanlama" icon={Timer}>
          <Row label="Dokunma eşiği" hint="Bundan kısa basışlar 'dokun' (aç/kapat), uzunları 'basılı tut' sayılır.">
            <Slider
              label="Dokunma eşiği"
              value={r.tapThresholdMs}
              min={120}
              max={1000}
              step={10}
              format={formatMs}
              onCommit={(v) => void update((d) => void (d.recording.tapThresholdMs = v))}
            />
          </Row>
          <Row label="Azami kayıt süresi" hint="Unutulan bir kaydın sonsuza dek sürmesini engeller.">
            <Slider
              label="Azami kayıt süresi"
              value={r.maxDurationS}
              min={10}
              max={1800}
              step={10}
              format={formatDuration}
              onCommit={(v) => void update((d) => void (d.recording.maxDurationS = v))}
            />
          </Row>
        </Card>

        <Card title="Sessizlik ve temizlik" icon={AudioLines}>
          <Row label="Sessizlikte otomatik bitir" hint="Yalnızca 'dokun' modunda: konuşmanız bitip sessizlik sürünce kaydı kendiliğinden bitirir.">
            <Toggle
              label="Sessizlikte otomatik bitir"
              checked={r.silenceAutoStop}
              onChange={(v) => void update((d) => void (d.recording.silenceAutoStop = v))}
            />
          </Row>
          {r.silenceAutoStop && (
            <Row label="Sessizlik süresi">
              <Slider
                label="Sessizlik süresi"
                value={r.silenceTimeoutMs}
                min={500}
                max={10000}
                step={250}
                format={formatSecondsShort}
                onCommit={(v) => void update((d) => void (d.recording.silenceTimeoutMs = v))}
              />
            </Row>
          )}
          <Row
            label="Halüsinasyon filtresi"
            hint="Whisper'ın sessizlikte uydurduğu 'Altyazı M.K.', 'İzlediğiniz için teşekkürler' gibi ifadeleri atar."
          >
            <Toggle
              label="Halüsinasyon filtresi"
              checked={settings.text.filterHallucinations}
              onChange={(v) => void update((d) => void (d.text.filterHallucinations = v))}
            />
          </Row>
        </Card>
      </div>
    </>
  );
}
