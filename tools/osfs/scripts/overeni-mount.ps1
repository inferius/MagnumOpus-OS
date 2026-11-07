# Rucni overeni ovladace osfs na pripojene jednotce (WinFsp).
#
# Pouziti (na NOVE naformatovanem obrazu s datasetem /projekt):
#   osfs format t.img --size 64M
#   osfs dataset create t.img /projekt
#   osfs mount t.img Q:              (v jinem okne)
#   pwsh tools/osfs/scripts/overeni-mount.ps1 -Drive Q:
#   osfs unmount Q:  a  osfs check t.img --scrub
#
# Overuje: sirotky (smazany otevreny soubor), prejmenovani otevreneho,
# sdileci rezim, zamek rozsahu, smazani pri zavreni, atomicke nahrazeni,
# kopii, zmenu velikosti pismen, presun do datasetu, mazani.
param([string]$Drive = 'Q:')
$ErrorActionPreference = 'Continue'
function T($name, $ok) { if ($ok) { "OK    $name" } else { "CHYBA $name" } }
Add-Type -Name K -Namespace W -MemberDefinition '[DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern bool MoveFileExW(string a, string b, int f);'
$enc = [Text.Encoding]::ASCII

# 1. Smazany, ale otevreny soubor se da dal cist (sirotek).
[IO.File]::WriteAllText("$Drive\sirotek.txt", 'obsah sirotka')
$h = [IO.File]::Open("$Drive\sirotek.txt", 'Open', 'Read', 'ReadWrite,Delete')
[IO.File]::Delete("$Drive\sirotek.txt")
$buf = New-Object byte[] 13; $n = $h.Read($buf, 0, 13)
T 'smazany otevreny soubor jde dal cist' ($enc.GetString($buf, 0, $n) -eq 'obsah sirotka')
T 'smazane jmeno zmizelo hned' (-not (Test-Path "$Drive\sirotek.txt"))
[IO.File]::WriteAllText("$Drive\sirotek.txt", 'novy soubor')
T 'na stejne jmeno jde hned vytvorit novy soubor' ((Get-Content "$Drive\sirotek.txt") -eq 'novy soubor')
$h.Close()

# 2. Prejmenovani otevreneho souboru: zapis starym handlem jde do prejmenovaneho.
[IO.File]::WriteAllText("$Drive\projekt\a.txt", 'AAA')
$h = [IO.File]::Open("$Drive\projekt\a.txt", 'Open', 'ReadWrite', 'ReadWrite,Delete')
Rename-Item "$Drive\projekt\a.txt" 'b.txt'
$h.Seek(0, 'End') | Out-Null; $h.Write($enc.GetBytes('BBB'), 0, 3); $h.Close()
T 'zapis pres handle po prejmenovani' ((Get-Content "$Drive\projekt\b.txt") -eq 'AAABBB')

# 3. Sdileci rezim (hlida WinFsp): FileShare.None -> druhe otevreni selze.
$h = [IO.File]::Open("$Drive\projekt\b.txt", 'Open', 'Read', 'None')
$denied = $false; try { $g = [IO.File]::Open("$Drive\projekt\b.txt", 'Open', 'Read', 'ReadWrite'); $g.Close() } catch { $denied = $true }
$h.Close()
T 'FileShare.None brani druhemu otevreni' $denied

# 4. Zamek rozsahu (LockFileEx): zapis z jineho handlu do zamceneho rozsahu selze.
$a = [IO.File]::Open("$Drive\projekt\b.txt", 'Open', 'ReadWrite', 'ReadWrite')
$b = [IO.File]::Open("$Drive\projekt\b.txt", 'Open', 'ReadWrite', 'ReadWrite')
$a.Lock(0, 3)
$blocked = $false; try { $b.Write($enc.GetBytes('X'), 0, 1); $b.Flush() } catch { $blocked = $true }
$a.Unlock(0, 3); $a.Close(); $b.Close()
T 'zamek rozsahu brani zapisu jineho handlu' $blocked

# 5. FILE_FLAG_DELETE_ON_CLOSE (primo CreateFileW; .NET DeleteOnClose ho nepreda).
Add-Type -Name F -Namespace W -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
public static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
[DllImport("kernel32.dll", SetLastError=true)]
public static extern bool CloseHandle(IntPtr h);
'@
$h = [W.F]::CreateFileW("$Drive\docasny.tmp", [Convert]::ToUInt32("C0010000", 16), 7, [IntPtr]::Zero, 2, [Convert]::ToUInt32("04000000", 16), [IntPtr]::Zero)
$exists = Test-Path "$Drive\docasny.tmp"
[W.F]::CloseHandle($h) | Out-Null
T 'smazat pri zavreni: existuje, dokud je otevreny' $exists
T 'smazat pri zavreni: po zavreni zmizel' (-not (Test-Path "$Drive\docasny.tmp"))

# 6. Nahrazeni (MoveFileEx REPLACE_EXISTING) je atomicke.
[IO.File]::WriteAllText("$Drive\cil.txt", 'stary'); [IO.File]::WriteAllText("$Drive\zdroj.txt", 'novy')
$ok = [W.K]::MoveFileExW("$Drive\zdroj.txt", "$Drive\cil.txt", 1)
T 'nahrazeni cile' ($ok -and (Get-Content "$Drive\cil.txt") -eq 'novy' -and -not (Test-Path "$Drive\zdroj.txt"))
$d = Get-ChildItem -Force "$Drive\" | Where-Object Name -like '.osfs*'
T 'po nahrazeni nezustal docasny soubor' ($null -eq $d)

# 7. Regrese: velky soubor, zmena velikosti pismen, presun do datasetu, mazani.
$big = New-Object byte[] (8MB); (New-Object Random 7).NextBytes($big); [IO.File]::WriteAllBytes("$env:TEMP\osfs-h.bin", $big)
Copy-Item "$env:TEMP\osfs-h.bin" "$Drive\big.bin"
T 'kopie 8 MiB se shodnym hashem' ((Get-FileHash "$Drive\big.bin").Hash -eq (Get-FileHash "$env:TEMP\osfs-h.bin").Hash)
Rename-Item "$Drive\big.bin" 'BIG.BIN'
T 'prejmenovani jen velikosti pismen' ((Get-ChildItem "$Drive\" -Filter 'big.bin').Name -ceq 'BIG.BIN')
Move-Item "$Drive\BIG.BIN" "$Drive\projekt\BIG.BIN"
T 'presun do datasetu (kopie + smazani)' ((Test-Path "$Drive\projekt\BIG.BIN") -and -not (Test-Path "$Drive\BIG.BIN"))
New-Item -ItemType Directory "$Drive\projekt\sub" | Out-Null; [IO.File]::WriteAllText("$Drive\projekt\sub\x.txt", 'x')
[IO.File]::Delete("$Drive\projekt\sub\x.txt"); [IO.Directory]::Delete("$Drive\projekt\sub")
T 'smazani souboru a prazdne slozky' (-not (Test-Path "$Drive\projekt\sub"))
$denied = $false; try { [IO.Directory]::Delete("$Drive\projekt") } catch { $denied = $true }
T 'slozku datasetu nejde smazat' $denied
"--- obsah:"; (Get-ChildItem -Recurse "$Drive\").FullName
