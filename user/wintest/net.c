/* Rezim site testu Windows vrstvy (W4b, docs/cs/windows-vrstva.md):
 * Winsock (ws2_32 -> wineserver -> nase sockety pres netd).
 *
 *   wine wintest.exe tcp <ip> <port>
 *
 * Spoji se, posle "ping-z-windows\n", precte odpoved (az 64 B, nejdel
 * 10 s) a vypise "tcp: ok <odpoved>" nebo "tcp: chyba <krok> <WSA kod>".
 * Kod konce 0 = spojeno a neco prislo zpet. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <winsock2.h>
#include <ws2tcpip.h>

int net_tcp(const char *ip, const char *port)
{
    WSADATA wsa;
    SOCKET s;
    struct sockaddr_in a;
    char buf[65];
    int n;
    DWORD timeout = 10000;
    static const char msg[] = "ping-z-windows\n";

    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0)
    {
        printf("tcp: chyba startup %d\n", WSAGetLastError());
        return 1;
    }
    s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (s == INVALID_SOCKET)
    {
        printf("tcp: chyba socket %d\n", WSAGetLastError());
        return 2;
    }
    memset(&a, 0, sizeof(a));
    a.sin_family = AF_INET;
    a.sin_port = htons((unsigned short)atoi(port));
    a.sin_addr.s_addr = inet_addr(ip);
    if (connect(s, (struct sockaddr *)&a, sizeof(a)) != 0)
    {
        printf("tcp: chyba connect %d\n", WSAGetLastError());
        return 3;
    }
    printf("tcp: spojeno\n");
    fflush(stdout);
    if (send(s, msg, (int)strlen(msg), 0) != (int)strlen(msg))
    {
        printf("tcp: chyba send %d\n", WSAGetLastError());
        return 4;
    }
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout, sizeof(timeout));
    n = recv(s, buf, sizeof(buf) - 1, 0);
    if (n <= 0)
    {
        printf("tcp: chyba recv %d %d\n", n, WSAGetLastError());
        return 5;
    }
    buf[n] = 0;
    while (n > 0 && (buf[n - 1] == '\n' || buf[n - 1] == '\r')) buf[--n] = 0;
    printf("tcp: ok %s\n", buf);
    fflush(stdout);
    closesocket(s);
    WSACleanup();
    return 0;
}
