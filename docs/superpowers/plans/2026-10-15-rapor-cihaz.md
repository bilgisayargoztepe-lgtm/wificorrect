# Raporlar cihaz panelinde — plan

Spec: `docs/superpowers/specs/2026-10-14-aylik-rapor-design.md` → "Ek (2026-10-15)". Dal `rapor-cihaz`.

- **Görev 1 (merkez):** "En çok ziyaret edilen alan adları" kaldırıldı (ham özet, rapor JSON'u, sayfa); biçim `bicim` 1 → 2
  (eski hamlar ve raporlar `wc-istatistik`in sonraki turunda bir kez yeniden üretilir; panel ve API o zamana kadar eski
  biçimi de okur, alan adı listesini hiç almaz). Cihaz API'si `/api/raporlar`, `/api/rapor` (aşağıda).
- **Görev 2 (cihaz):** cihaz paneli Ayarlar → **Raporlar** kutucuğu; istek anında merkezden alır, cihaz panelinin
  görünümüyle müşteri panelindeki bölümleri gösterir. Merkeze bağlı değilse ya da ulaşılamazsa "Raporlar şu an alınamıyor".
  Görüntüleme denetime yazılmaz.

## API sözleşmesi

`https://api.wificorrect.com`, mevcut cihaz uç noktalarıyla aynı düzen:

- Yalnızca `POST`, gövde JSON nesnesi, en çok 4096 bayt; IP başına dakikada 20 istek (bütün `/api/*` ortak).
- Kimlik: gövdede `cihaz_anahtari` (eşleşmede verilen anahtar, `/api/saglik` gibi).
- Yetki (`/api/paket`, `/api/liste` ile aynı kural): cihaz **bağlı** (`durum = bagli`; serbest bırakılıyor değil) ve müşterinin
  hizmeti açık (üyelik `aktif`, askıda değil, lisans süresi dolmamış). Cihaz yalnızca **kendi müşterisinin**
  (`rapor/<numara>/`) raporlarını görür; numara gövdeden alınmaz, anahtardan bulunur.
- Cevap başlıkları: `Content-Type: application/json; charset=utf-8`, `Cache-Control: no-store`.
- Merkez hareket (denetim) satırı yazmaz (raporda kişisel veri yok).

### Hatalar

Gövde: `{"hata": "<Türkçe metin>", "kod": "<kod>"}`.

| HTTP | kod | Ne zaman |
|---|---|---|
| 400 | `gecersiz` | Gövde JSON nesnesi değil; `/api/rapor`'da `tur`/`donem` geçersiz |
| 401 | `taninmadi` | `cihaz_anahtari` yok ya da tanınmıyor |
| 403 | `yetki` | Cihaz bağlı değil (serbest bırakılıyor) ya da üyelik/lisans kapalı, askıda |
| 404 | `yok` | `/api/rapor`: o dönemin dosyası yok ya da bozuk |
| 413 | `gecersiz` | Gövde > 4096 bayt |
| 429 | `hiz` | IP başına dakikada 20 istek aşıldı |

Cihaz, 200 dışındaki her durumda (ve ağ hatası/zaman aşımında) "Raporlar şu an alınamıyor" gösterebilir; 404'te
"Bu dönem için rapor yok" daha doğrudur.

### `POST /api/raporlar`

İstek: `{"cihaz_anahtari": "…"}`

Cevap 200:

```json
{
  "raporlar": [
    {"tur": "hafta", "donem": "2026-W42", "baslangic": "2026-10-12", "bitis": "2026-10-18",
     "kesin": false, "farkli": 14, "kayit_gun": 2, "kayit_tam": 2},
    {"tur": "ay", "donem": "2026-10", "baslangic": "2026-10-01", "bitis": "2026-10-31",
     "kesin": false, "farkli": 41, "kayit_gun": 13, "kayit_tam": 12}
  ],
  "bugun": "2026-10-14"
}
```

| Alan | Tür | Anlam |
|---|---|---|
| `raporlar` | dizi (0–100) | Önce haftalar, sonra aylar; her grup yeniden eskiye. Rapor yoksa `[]` ("Henüz rapor yok") |
| `raporlar[].tur` | `"hafta"` \| `"ay"` | |
| `raporlar[].donem` | metin | hafta `YYYY-Www` (ISO yılı, `W01`–`W53`), ay `YYYY-MM` |
| `raporlar[].baslangic`, `.bitis` | metin `YYYY-MM-DD` | Dönemin ilk ve son günü (hafta Pzt–Paz, TR saati) |
| `raporlar[].kesin` | bool | `false`: geç gelen günlerle değişebilir |
| `raporlar[].farkli` | tamsayı ≥ 0 | Farklı misafir |
| `raporlar[].kayit_gun`, `.kayit_tam` | tamsayı ≥ 0 | 5651: bitmiş (bugünden önceki, hizmet öncesi hariç) gün sayısı ve bunlardan "tam" olanlar |
| `bugun` | metin `YYYY-MM-DD` | Merkezin TR tarihi. "sürüyor" etiketi: `bitis >= bugun` |

Etiket kuralı (müşteri paneliyle aynı): `bitis >= bugun` → "sürüyor"; değilse `kesin == false` → "kesinleşmedi".
Özet metni: `kayit_gun == 0` → "Henüz biten gün yok", değilse "N günün M'i tam" (Türkçe iyelik eki: `raporlar.iyelik`).

### `POST /api/rapor`

İstek: `{"cihaz_anahtari": "…", "tur": "ay", "donem": "2026-10"}`

`tur` ∈ {`hafta`, `ay`}; `donem` hafta için tam olarak `\d{4}-W\d{2}` (hafta 01–53), ay için `\d{4}-\d{2}` (ay 01–12),
ASCII rakam. Başka her değer (sayı, `null`, `../x`, küçük `w`, `2026-9`) → 400.

Cevap 200 (dört üst alan: `rapor`, `isletme`, `cihaz`, `bugun`):

```json
{
  "rapor": {
    "bicim": 2, "tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30",
    "uretim": "2026-09-21T12:31:07+03:00", "kesin": false,
    "misafir": {"farkli": 41, "yeni": 30, "donen": 11, "giris": 77, "giris_sms": 70, "giris_kod": 7,
                "izinli_cihaz": 2, "gunluk_ortalama": 4.5, "en_yogun_gun": {"gun": "2026-09-12", "farkli": 9}},
    "saat": [0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 9, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    "hafta_gunu": [1, 2, 3, 4, 5, 6, 7],
    "gunler": [
      {"gun": "2026-09-01", "durum": "tam", "neden": "", "farkli": 2},
      {"gun": "2026-09-04", "durum": "dogrulanamadi", "neden": "zincir_yok", "farkli": 2},
      {"gun": "2026-09-05", "durum": "kayit_yok", "neden": "", "farkli": null}
    ],
    "kayit": {"gun": 20, "tam": 16},
    "kullanim": {"gonderilen": 1073741824, "alinan": 3221225472, "sms": 70},
    "surum": "0.4.9", "surum_sonra_degisti": false
  },
  "isletme": {"numara": "1537344", "ad": "Göztepe Bilgisayar", "unvan": "Göztepe Bilgisayar Ltd. Şti."},
  "cihaz": {
    "surum": "0.4.9",
    "cevrimdisi_sn": 14400,
    "sorunlar": [
      {"tur": "servis", "onem": "kritik", "metin": "Servis durdu: portal",
       "acilis": "2026-09-10T08:00:00+03:00", "kapanis": "2026-09-10T09:30:00+03:00", "sure_sn": 5400},
      {"tur": "ulasilamiyor", "onem": "kritik", "metin": "Cihaz ulaşılamıyor",
       "acilis": "2026-09-21T15:13:20+03:00", "kapanis": null, "sure_sn": 7200}
    ]
  },
  "bugun": "2026-09-21"
}
```

**`rapor`** — merkezin doğruladığı dönem raporu (`raporlar.normal_rapor`; her alan tür denetimli, bozuk alan 0/boş olur):

| Alan | Tür | Anlam |
|---|---|---|
| `bicim` | tamsayı | Şu an `2` |
| `tur`, `donem`, `baslangic`, `bitis` | metin | İstekteki dönem; tarihler `YYYY-MM-DD` |
| `uretim` | metin | Raporun üretildiği an, ISO `+03:00` (boş olabilir) |
| `kesin` | bool | `false`: geç gelen günlerle değişebilir |
| `misafir.farkli`, `.yeni`, `.donen` | tamsayı ≥ 0 | Farklı misafir; ilk kez gelen; daha önce de gelmiş |
| `misafir.giris`, `.giris_sms`, `.giris_kod` | tamsayı ≥ 0 | Toplam giriş (OTURUM_BASLA) = SMS + misafir kodu |
| `misafir.izinli_cihaz` | tamsayı ≥ 0 | Portalsız izinli cihaz |
| `misafir.gunluk_ortalama` | sayı (ondalıklı) ≥ 0 | Günlük ortalama farklı misafir (gösterim: 1 hane, virgül) |
| `misafir.en_yogun_gun` | `{"gun": "YYYY-MM-DD", "farkli": int}` \| `null` | |
| `saat` | 24 tamsayı | Saat 0–23 (TR) için o saatte internete çıkan farklı misafir |
| `hafta_gunu` | 7 tamsayı | Pazartesi…Pazar farklı misafir |
| `gunler` | dizi | Dönemin her günü, sırayla |
| `gunler[].gun` | metin `YYYY-MM-DD` | |
| `gunler[].durum` | metin | `tam`, `kayit_yok`, `eksik`, `dogrulanamadi`, `bekleniyor`, `bugun`, `gelecek`, `hizmet_oncesi`, `bilinmiyor` |
| `gunler[].neden` | metin | Yalnızca `dogrulanamadi` için: `okunamadi`, `dosya`, `zincir_yok`, `zincir`, `manifest`; değilse `""` |
| `gunler[].farkli` | tamsayı ≥ 0 \| `null` | O günün farklı misafiri; ham özet yoksa `null` (boş hücre) |
| `kayit.gun`, `kayit.tam` | tamsayı ≥ 0 | "N günün M'i tam" |
| `kullanim.gonderilen`, `.alinan` | tamsayı ≥ 0 (bayt) | Yükleme / indirme; toplam veri = ikisinin toplamı |
| `kullanim.sms` | tamsayı ≥ 0 | Gönderilen doğrulama SMS'i |
| `surum` | metin \| `null` | Dönem sonundaki sürüm (arşivdeki güncelleme olaylarından); gösterim için `cihaz.surum` kullanın |
| `surum_sonra_degisti` | bool | |

Alan adı listesi (`kullanim.alanlar`) **yoktur**.

**`isletme`** — rapor başlığı:

| Alan | Tür | Anlam |
|---|---|---|
| `numara` | metin (7 hane) | Müşteri no |
| `ad` | metin | Bağlı cihazın bildirdiği işletme adı; boşsa `"Müşteri <numara>"` |
| `unvan` | metin | Boş olabilir (gösterim "—") |

**`cihaz`** — "Cihaz" bölümü (müşteri panelinin gösterdiğiyle aynı hesap, `raporlar.cihaz_bolumu`):

| Alan | Tür | Anlam |
|---|---|---|
| `surum` | metin \| `null` | Gösterilecek "Yazılım sürümü (dönem sonu)": raporun sürümü; yoksa ve sonradan değişmediyse cihazın merkezdeki bugünkü sürümü; yoksa `null` ("—") |
| `cevrimdisi_sn` | tamsayı ≥ 0 | Dönemle (ve şimdiye kadarla) kesişen `ulasilamiyor` sorun süreleri toplamı. `0` → "Yok"; değilse "en az <süre>" |
| `sorunlar` | dizi | Dönem içinde **açılan** kritik/uyarı sorunlar (bilgi düzeyi, ör. lisans, yok), açılışa göre sıralı. Boşsa "Bu dönemde açılan sorun yok." |
| `sorunlar[].tur` | metin | Sorun türü (`ulasilamiyor`, `servis`, `disk`, …) |
| `sorunlar[].onem` | `"kritik"` \| `"uyari"` | Gösterim "Kritik" / "Uyarı" |
| `sorunlar[].metin` | metin | Merkezin sorun metni (Türkçe). **HTML'de kaçışlanarak** basılır |
| `sorunlar[].acilis` | metin | ISO `+03:00` |
| `sorunlar[].kapanis` | metin \| `null` | `null` → hâlâ açık ("açık") |
| `sorunlar[].sure_sn` | tamsayı ≥ 0 \| `null` | Kapanış (ya da şimdi) − açılış; açılış okunamazsa `null` ("—") |

Süre gösterimi (`raporlar.sure_metni`): < 60 dk "N dk", < 1 gün "S sa D dk", değilse "G gün S sa".
Çevrimdışı notu: "Çevrimdışı süre merkezin cihaza ulaşamadığını fark ettiği andan sayılır; gerçek süre biraz daha uzun olabilir."

**`bugun`** — merkezin TR tarihi (`YYYY-MM-DD`); "Dönem sürüyor" = `rapor.bitis >= bugun`.

### Cihaz tarafı için notlar

- Cevaptaki her metin cihaz için güvenilmez girdidir: türünü denetleyin, HTML'e kaçışlayarak basın (merkez zaten doğrular).
- Merkezde rapor dosyaları günde iki kez (02:30, 12:30) güncellenir; cihazda önbellek gerekmez.
- `/api/*` IP başına dakikada 20 istek: liste + birkaç rapor açmak sınırın altında kalır; sağlık raporu (5 dk) da bu
  sayaca girer.
