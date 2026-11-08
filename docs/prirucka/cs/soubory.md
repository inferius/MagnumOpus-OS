# Soubory a verze

Správce souborů s okny a ikonami *připravujeme*. Se soubory zatím pracujete
v **Terminálu** (viz [Terminál pro začátečníky](terminal.md)).

## Vrácení změn

Souborový systém si pamatuje změny, takže chyby jdou vrátit:

- **`undo`** vrátí poslední příkaz, který měnil soubory. Opakovaně až
  16 příkazů zpět.
- **`preview { … }`** provede příkazy jen „na zkoušku“ (bokem), ukáže
  tabulku změn a převezme je až po vašem potvrzení. Když odmítnete, nic
  se nezmění.
- Příkaz, který by změnil víc než 20 souborů (mimo `/tmp`), se vždy
  nejdřív zeptá.

Koš, historii verzí jednoho souboru a „zkusit bokem“ v okně správce
souborů *připravujeme*.
