<#
.SYNOPSIS
    WifiCorrect: müşterinin AP'sinin web arayüzünü cihaz üzerinden (SSH SOCKS tüneli) ayrı bir Chrome penceresinde açar.

.DESCRIPTION
    Cihaza "ssh -N -D" ile SOCKS tüneli açar, geçici profilli bir Chrome (yoksa Edge) penceresini bu proxy ile ve verilen
    AP adresiyle açar; pencere kapanınca tüneli kapatır ve geçici profili siler. Normal Chrome profiline dokunmaz.
    Ad çözümü SOCKS5'te cihazda yapılır. Sır içermez: kullanıcının mevcut SSH anahtarı kullanılır (BatchMode: parola
    sorulmaz; anahtar parolalıysa ssh-agent'a eklenmiş olmalı). Ayrıntı: docs/AP_KURULUM.md.
    Windows PowerShell 5.1 ile çalışır.

.EXAMPLE
    scripts\ap-ac.cmd -Adres 10.50.0.2
.EXAMPLE
    scripts\ap-ac.cmd -Adres 192.168.0.254 -Cihaz root@10.99.0.11
.EXAMPLE
    scripts\ap-ac.cmd -Adres http://10.50.0.2/userRpm/Index.htm -Kuru
#>
[CmdletBinding()]
param(
    # AP adresi: 10.50.0.2, 192.168.0.254, tplinkap.net ya da http(s)://adres[:port][/yol]
    [Parameter(Mandatory = $true)][string]$Adres,
    # SSH hedefi: ~/.ssh/config girdisi (dükkân ağında "wificorrect") ya da dışarıdan "root@10.99.0.11"
    [string]$Cihaz = 'wificorrect',
    # Yerel SOCKS portu; doluysa 1081-1099 arasında boş olan seçilir
    [int]$Port = 1080,
    # İsteğe bağlı SSH anahtar dosyası (ssh -i)
    [string]$Anahtar = '',
    # Yalnızca komutları yazar, hiçbir şey çalıştırmaz
    [switch]$Kuru
)

$ErrorActionPreference = 'Stop'

function Hata([string]$Mesaj) {
    Write-Host "HATA: $Mesaj" -ForegroundColor Red
    exit 1
}

# ---------------------------------------------------------------- girdileri denetle
$Adres = $Adres.Trim()
# -cnotmatch: büyük/küçük harf duyarlı (sınıf ikisini de içerir); duyarsız eşleşme Türkçe yerel ayarda I/ı yüzünden şaşar.
$adresKalibi = '^(https?://)?[A-Za-z0-9]([A-Za-z0-9.-]{0,252})(:[0-9]{1,5})?(/[A-Za-z0-9._~/%?=&#+-]*)?$'
if ($Adres -cnotmatch $adresKalibi) {
    Hata "Adres geçersiz: '$Adres'. Örnek: 10.50.0.2, 192.168.0.254, http://10.50.0.2/ (boşluk, tırnak, ; gibi karakterler olmaz)."
}
if ($Adres -match '^https?://') {
    $url = $Adres
} else {
    $url = "http://$Adres"
}
if ($url -notmatch '^https?://[^/]+/') {
    $url = "$url/"
}
if ($Cihaz -cnotmatch '^[A-Za-z0-9._-]+(@[A-Za-z0-9._-]+)?$') {
    Hata "Cihaz geçersiz: '$Cihaz'. Örnek: wificorrect ya da root@10.99.0.11."
}
if ($Port -lt 1024 -or $Port -gt 65535) {
    Hata "Port 1024-65535 arasında olmalı."
}
if ($Anahtar -ne '') {
    if ($Anahtar.Contains('"')) {
        Hata 'Anahtar yolunda tırnak olamaz.'
    }
    if (-not $Kuru -and -not (Test-Path -LiteralPath $Anahtar -PathType Leaf)) {
        Hata "Anahtar dosyası bulunamadı: $Anahtar"
    }
}

# ---------------------------------------------------------------- boş port
function PortBos([int]$P) {
    $dinleyici = $null
    try {
        $dinleyici = New-Object System.Net.Sockets.TcpListener ([System.Net.IPAddress]::Loopback), $P
        $dinleyici.Start()
        return $true
    } catch {
        return $false
    } finally {
        if ($null -ne $dinleyici) {
            try { $dinleyici.Stop() } catch { }
        }
    }
}

$adaylar = @($Port) + @(1081..1099 | Where-Object { $_ -ne $Port })
$secilen = $null
foreach ($p in $adaylar) {
    if (PortBos $p) {
        $secilen = $p
        break
    }
}
if ($null -eq $secilen) {
    Hata "Boş yerel port bulunamadı ($Port, 1081-1099 dolu)."
}
$Port = $secilen

# ---------------------------------------------------------------- programlar
$sshKomut = Get-Command ssh.exe -ErrorAction SilentlyContinue
if ($null -ne $sshKomut) {
    $sshYolu = $sshKomut.Source
} elseif ($Kuru) {
    $sshYolu = 'ssh.exe'
} else {
    Hata 'ssh.exe bulunamadı. Windows Ayarlar > Uygulamalar > İsteğe bağlı özellikler > "OpenSSH İstemcisi" kurulmalı.'
}

$tarayiciAdaylari = @()
foreach ($kok in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:LOCALAPPDATA)) {
    if ($kok) {
        $tarayiciAdaylari += (Join-Path $kok 'Google\Chrome\Application\chrome.exe')
    }
}
foreach ($kok in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
    if ($kok) {
        $tarayiciAdaylari += (Join-Path $kok 'Microsoft\Edge\Application\msedge.exe')
    }
}
$tarayici = $null
foreach ($t in $tarayiciAdaylari) {
    if (Test-Path -LiteralPath $t -PathType Leaf) {
        $tarayici = $t
        break
    }
}
if ($null -eq $tarayici) {
    if ($Kuru) {
        $tarayici = 'chrome.exe'
    } else {
        Hata 'Chrome ya da Edge bulunamadı (Program Files, Program Files (x86), AppData\Local).'
    }
}

$profil = Join-Path $env:TEMP ('wfc-ap-' + [guid]::NewGuid().ToString('N').Substring(0, 8))

# Start-Process (PowerShell 5.1) argümanları boşlukla birleştirir, tırnak eklemez: boşluk içerebilen yollar elle tırnaklanır.
$sshArg = @('-N', '-D', "127.0.0.1:$Port", '-o', 'ExitOnForwardFailure=yes', '-o', 'ServerAliveInterval=15', '-o', 'BatchMode=yes')
if ($Anahtar -ne '') {
    $sshArg += @('-i', "`"$Anahtar`"")
}
$sshArg += $Cihaz
$tarayiciArg = @("`"--user-data-dir=$profil`"", "--proxy-server=socks5://127.0.0.1:$Port", '--no-first-run',
    '--no-default-browser-check', '--new-window', $url)

if ($Kuru) {
    Write-Host "ssh     : `"$sshYolu`" $($sshArg -join ' ')"
    Write-Host "tarayıcı: `"$tarayici`" $($tarayiciArg -join ' ')"
    exit 0
}

# ---------------------------------------------------------------- tünel + tarayıcı
$ssh = $null
try {
    Write-Host "SSH tüneli açılıyor: $Cihaz -> 127.0.0.1:$Port ..."
    $ssh = Start-Process -FilePath $sshYolu -ArgumentList $sshArg -WindowStyle Hidden -PassThru
    $hazir = $false
    for ($i = 0; $i -lt 30; $i++) {
        if ($ssh.HasExited) {
            break
        }
        $istemci = New-Object System.Net.Sockets.TcpClient
        try {
            $istemci.Connect('127.0.0.1', $Port)
            $hazir = $true
        } catch {
            Start-Sleep -Milliseconds 500
        } finally {
            $istemci.Close()
        }
        if ($hazir) {
            break
        }
    }
    if (-not $hazir) {
        if ($ssh.HasExited) {
            Hata "SSH tüneli açılamadı (ssh çıkış kodu $($ssh.ExitCode)): cihaz adı/anahtar/ağ (WireGuard açık mı?). Elle deneyin: ssh $Cihaz"
        }
        Hata 'SSH tüneli 15 sn içinde açılmadı: cihaz adı/anahtar/ağ (WireGuard açık mı?).'
    }
    New-Item -ItemType Directory -Path $profil -Force | Out-Null
    Write-Host "Tarayıcı açılıyor: $url (pencere kapanınca tünel kapanır)"
    $pencere = Start-Process -FilePath $tarayici -ArgumentList $tarayiciArg -PassThru
    $pencere.WaitForExit()
} finally {
    if ($null -ne $ssh -and -not $ssh.HasExited) {
        try { Stop-Process -Id $ssh.Id -Force -ErrorAction Stop } catch { Write-Warning "ssh süreci durdurulamadı (PID $($ssh.Id))." }
    }
    if (Test-Path -LiteralPath $profil) {
        $silindi = $false
        for ($i = 0; $i -lt 5; $i++) {
            try {
                Remove-Item -LiteralPath $profil -Recurse -Force -ErrorAction Stop
                $silindi = $true
                break
            } catch {
                Start-Sleep -Seconds 1
            }
        }
        if (-not $silindi) {
            Write-Warning "Geçici profil silinemedi: $profil (elle silebilirsiniz)."
        }
    }
    Write-Host 'Tünel kapandı.'
}
