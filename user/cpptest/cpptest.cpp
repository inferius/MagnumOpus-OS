// Test behu C++ (krok 7a-1, docs/cs/cpp.md): libc++, libc++abi, libunwind
// nad cstd. Staticky (`cpptest`) i nad libc++.so (`cpptest-dyn`).
//
// Kazda cast vypise "ok <jmeno>", na konci "cpptest: hotovo" (protokol
// testu initu, user/init/src/tests/cpp.rs). Chyba = zprava na stderr
// a kod 1. Argument 1 = adresar pro std::filesystem.
//
// Od 7a-2 i proudy (<iostream>, <fstream>, <sstream>), locale, siroke znaky,
// <format> a <regex>.

#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <filesystem>
#include <format>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <locale>
#include <regex>
#include <sstream>
#include <functional>
#include <map>
#include <memory>
#include <mutex>
#include <new>
#include <optional>
#include <random>
#include <stdexcept>
#include <string>
#include <thread>
#include <typeinfo>
#include <unordered_map>
#include <cwchar>
#include <cwctype>
#include <variant>
#include <vector>

extern "C" int cframe_call(void (*cb)(void *), void *arg);
extern "C" int cframe_after(void);

static int failures;

#define CHECK(cond)                                                                                                    \
    do {                                                                                                               \
        if (!(cond)) {                                                                                                 \
            std::fprintf(stderr, "cpptest: selhalo %s (%s:%d)\n", #cond, __FILE__, __LINE__);                          \
            failures++;                                                                                                \
            return;                                                                                                    \
        }                                                                                                              \
    } while (0)

static void ok(const char *name)
{
    std::printf("ok %s\n", name);
}

// --- staticke konstruktory a destruktory ----------------------------------

static int global_ctor_ran;

struct Global {
    Global() { global_ctor_ran = 42; }
    ~Global()
    {
        // Po main (atexit / __cxa_atexit): vypis az za "hotovo".
        std::printf("cpptest: destruktor globalniho objektu\n");
    }
};

static Global global_object;

static void test_static_init()
{
    CHECK(global_ctor_ran == 42);
    ok("staticke-konstruktory");
}

// --- kontejnery a algoritmy -----------------------------------------------

static void test_containers()
{
    std::vector<int> v;
    for (int i = 0; i < 1000; i++)
        v.push_back((i * 7919) % 1000);
    std::sort(v.begin(), v.end());
    CHECK(std::is_sorted(v.begin(), v.end()));
    CHECK(v.front() == 0 && v.back() == 999);

    std::map<std::string, int> m;
    m["jedna"] = 1;
    m["dva"] = 2;
    m["tri"] = 3;
    CHECK(m.size() == 3 && m.begin()->first == "dva");

    std::unordered_map<int, std::string> u;
    for (int i = 0; i < 500; i++)
        u[i] = std::to_string(i);
    CHECK(u[123] == "123" && u.size() == 500);

    std::string s = "Ahoj";
    s += ", svete";
    s.append(100, '!');
    CHECK(s.size() == 111 && s.find("svete") == 6);

    std::optional<int> o = 5;
    std::variant<int, std::string> var = std::string("text");
    CHECK(o.value() == 5 && std::get<std::string>(var) == "text");

    auto sp = std::make_shared<std::vector<int>>(10, 3);
    std::weak_ptr<std::vector<int>> wp = sp;
    CHECK(wp.lock()->at(9) == 3);
    sp.reset();
    CHECK(wp.expired());

    std::function<int(int)> f = [k = 3](int x) { return x * k; };
    CHECK(f(5) == 15);
    ok("kontejnery");
}

// --- vyjimky ---------------------------------------------------------------

struct MyError : std::runtime_error {
    int code;
    explicit MyError(int c) : std::runtime_error("moje chyba"), code(c) {}
};

static int destroyed;

struct Guard {
    ~Guard() { destroyed++; }
};

static void thrower(int depth)
{
    Guard g;
    if (depth == 0)
        throw MyError(17);
    thrower(depth - 1);
}

static void test_exceptions()
{
    destroyed = 0;
    try {
        thrower(5);
        CHECK(false);
    } catch (const MyError &e) {
        CHECK(e.code == 17 && std::strcmp(e.what(), "moje chyba") == 0);
    }
    CHECK(destroyed == 6);

    // Zachyceni pres bazovou tridu a znovuvyhozeni.
    try {
        try {
            throw std::out_of_range("mimo");
        } catch (const std::logic_error &) {
            throw;
        }
    } catch (const std::exception &e) {
        CHECK(std::strcmp(e.what(), "mimo") == 0);
    }

    // std::exception_ptr mezi bloky.
    std::exception_ptr ep;
    try {
        throw 123;
    } catch (...) {
        ep = std::current_exception();
    }
    try {
        std::rethrow_exception(ep);
    } catch (int x) {
        CHECK(x == 123);
    }

    // Vyjimka z knihovny (vector::at).
    std::vector<int> v(3);
    bool caught = false;
    try {
        (void)v.at(10);
    } catch (const std::out_of_range &) {
        caught = true;
    }
    CHECK(caught);
    ok("vyjimky");
}

static void callback_throws(void *arg)
{
    Guard g;
    throw std::string(static_cast<const char *>(arg));
}

static void test_exception_through_c()
{
    destroyed = 0;
    try {
        cframe_call(callback_throws, (void *)"pres C");
        CHECK(false);
    } catch (const std::string &s) {
        CHECK(s == "pres C");
    }
    CHECK(destroyed == 1);
    CHECK(cframe_after() == 0);
    ok("vyjimka-pres-ramec-c");
}

// --- RTTI ------------------------------------------------------------------

struct Base {
    virtual ~Base() = default;
    virtual int id() const { return 1; }
};
struct Derived : Base {
    int id() const override { return 2; }
};
struct Other : Base {};

static void test_rtti()
{
    std::unique_ptr<Base> b = std::make_unique<Derived>();
    CHECK(b->id() == 2);
    CHECK(dynamic_cast<Derived *>(b.get()) != nullptr);
    CHECK(dynamic_cast<Other *>(b.get()) == nullptr);
    Base &ref = *b;
    CHECK(typeid(ref) == typeid(Derived));
    CHECK(std::strstr(typeid(Derived).name(), "Derived") != nullptr);
    bool bad_cast = false;
    try {
        (void)dynamic_cast<Other &>(ref);
    } catch (const std::bad_cast &) {
        bad_cast = true;
    }
    CHECK(bad_cast);
    ok("rtti");
}

// --- pamet -----------------------------------------------------------------

struct alignas(256) Wide {
    char data[300];
};

static void test_memory()
{
    auto w = std::make_unique<Wide>();
    CHECK(reinterpret_cast<uintptr_t>(w.get()) % 256 == 0);
    std::vector<Wide> many(5);
    CHECK(reinterpret_cast<uintptr_t>(many.data()) % 256 == 0);
    bool bad_alloc = false;
    try {
        std::vector<char> huge(std::size_t(1) << 46);
        (void)huge;
    } catch (const std::bad_alloc &) {
        bad_alloc = true;
    } catch (const std::length_error &) {
        bad_alloc = true;
    }
    CHECK(bad_alloc);
    void *p = ::operator new(64, std::align_val_t(4096), std::nothrow);
    CHECK(p != nullptr && reinterpret_cast<uintptr_t>(p) % 4096 == 0);
    ::operator delete(p, std::align_val_t(4096));
    ok("pamet");
}

// --- vlakna ----------------------------------------------------------------

static std::atomic<int> tls_destroyed;

struct PerThread {
    int value = 0;
    ~PerThread() { tls_destroyed++; }
};

static thread_local PerThread per_thread;

static void test_threads()
{
    std::mutex mu;
    std::condition_variable cv;
    int ready = 0;
    long sum = 0;
    std::atomic<int> counter{0};
    tls_destroyed = 0;
    std::vector<std::thread> ts;
    for (int t = 0; t < 4; t++) {
        ts.emplace_back([&, t] {
            per_thread.value = t + 1;
            for (int i = 0; i < 10000; i++)
                counter.fetch_add(1, std::memory_order_relaxed);
            std::lock_guard<std::mutex> l(mu);
            sum += per_thread.value;
            ready++;
            cv.notify_one();
        });
    }
    {
        std::unique_lock<std::mutex> l(mu);
        cv.wait(l, [&] { return ready == 4; });
    }
    for (auto &t : ts)
        t.join();
    CHECK(counter.load() == 40000);
    CHECK(sum == 1 + 2 + 3 + 4);
    // Destruktory thread_local probehly pri konci kazdeho vlakna.
    CHECK(tls_destroyed.load() == 4);
    // Vyjimka uvnitr vlakna, zachycena tamtez.
    bool caught = false;
    std::thread([&] {
        try {
            throw std::runtime_error("ve vlakne");
        } catch (const std::exception &) {
            caught = true;
        }
    }).join();
    CHECK(caught);
    CHECK(std::thread::hardware_concurrency() >= 1);
    ok("vlakna");
}

// --- cas a nahoda ---------------------------------------------------------

static void test_chrono_random()
{
    using namespace std::chrono;
    auto a = steady_clock::now();
    std::this_thread::sleep_for(milliseconds(20));
    auto d = duration_cast<milliseconds>(steady_clock::now() - a).count();
    CHECK(d >= 19);
    auto now = system_clock::to_time_t(system_clock::now());
    CHECK(now > 1600000000);

    std::random_device rd;
    unsigned x = rd(), y = rd(), z = rd();
    CHECK(!(x == y && y == z));
    std::mt19937 gen(rd());
    std::uniform_int_distribution<int> dist(1, 6);
    int r = dist(gen);
    CHECK(r >= 1 && r <= 6);
    ok("cas-nahoda");
}

// --- std::filesystem nad fsd ----------------------------------------------

static void test_filesystem(const char *dir)
{
    namespace fs = std::filesystem;
    fs::path base = fs::path(dir) / "strom";
    std::error_code ec;
    fs::remove_all(base, ec);
    CHECK(fs::create_directories(base / "a" / "b"));
    CHECK(fs::is_directory(base / "a" / "b"));
    {
        FILE *f = std::fopen((base / "a" / "soubor.txt").c_str(), "w");
        CHECK(f != nullptr);
        std::fputs("obsah souboru", f);
        std::fclose(f);
    }
    CHECK(fs::file_size(base / "a" / "soubor.txt") == 13);
    fs::rename(base / "a" / "soubor.txt", base / "prejmenovany.txt");
    CHECK(fs::exists(base / "prejmenovany.txt") && !fs::exists(base / "a" / "soubor.txt"));
    int entries = 0;
    for (auto &e : fs::recursive_directory_iterator(base)) {
        (void)e;
        entries++;
    }
    CHECK(entries == 3);
    bool threw = false;
    try {
        (void)fs::file_size(base / "neexistuje");
    } catch (const fs::filesystem_error &) {
        threw = true;
    }
    CHECK(threw);
    CHECK(fs::remove_all(base) == 4);
    CHECK(!fs::exists(base));
    ok("filesystem");
}

// --- proudy, locale, siroke znaky (7a-2) -----------------------------------

static void test_streams(const char *dir)
{
    std::ostringstream os;
    os << 42 << ' ' << std::hex << 255 << ' ' << std::dec << std::fixed << std::setprecision(3) << 3.14159;
    CHECK(os.str() == "42 ff 3.142");
    std::istringstream is("17 2.5 slovo");
    int i = 0;
    double d = 0;
    std::string w;
    is >> i >> d >> w;
    CHECK(i == 17 && d == 2.5 && w == "slovo");
    std::cout << "ok iostream" << std::endl;
    if (dir) {
        std::string path = std::string(dir) + "/proud.txt";
        {
            std::ofstream out(path);
            CHECK(out.good());
            out << "radek 1\n" << 12345 << "\n";
        }
        std::ifstream in(path);
        std::string line;
        CHECK(std::getline(in, line) && line == "radek 1");
        long n = 0;
        in >> n;
        CHECK(n == 12345);
    }
    ok("proudy");
}

static void test_locale_wide()
{
    std::locale loc("C.UTF-8");
    CHECK(loc.name() == "C.UTF-8");
    auto &ct = std::use_facet<std::ctype<wchar_t>>(loc);
    // U+010D -> U+010C, U+017E -> U+017D
    CHECK(ct.toupper(L'\u010d') == L'\u010c');
    CHECK(ct.is(std::ctype_base::alpha, L'\u017e'));
    // UTF-8 -> wchar_t pres codecvt facetu: "zlut" s hacky = 4 znaky.
    const char utf8[] = "\xc5\xbelu\xc5\xa5";
    std::mbstate_t st{};
    wchar_t out[8];
    const char *from_next = nullptr;
    wchar_t *to_next = nullptr;
    auto &cv = std::use_facet<std::codecvt<wchar_t, char, std::mbstate_t>>(loc);
    auto r = cv.in(st, utf8, utf8 + sizeof(utf8) - 1, from_next, out, out + 8, to_next);
    CHECK(r == std::codecvt_base::ok && to_next - out == 4 && out[0] == L'\u017e' && out[3] == L'\u0165');
    std::wstring ws = L"a\u010d";
    CHECK(ws.size() == 2 && std::wcslen(ws.c_str()) == 2);
    std::wostringstream wos;
    wos << L"cislo " << 7;
    CHECK(wos.str() == L"cislo 7");
    ok("locale-siroke-znaky");
}

static void test_format_regex()
{
    CHECK(std::format("{:>5}|{:.2f}|{:#x}", 42, 3.14159, 255) == "   42|3.14|0xff");
    std::regex re("([a-z]+)-([0-9]+)");
    std::smatch m;
    std::string text = "verze abc-123 konec";
    CHECK(std::regex_search(text, m, re) && m[1] == "abc" && m[2] == "123");
    ok("format-regex");
}

int main(int argc, char **argv)
{
    test_static_init();
    test_containers();
    test_exceptions();
    test_exception_through_c();
    test_rtti();
    test_memory();
    test_threads();
    test_chrono_random();
    if (argc > 1)
        test_filesystem(argv[1]);
    test_streams(argc > 1 ? argv[1] : nullptr);
    test_locale_wide();
    test_format_regex();
    if (failures)
        return 1;
    std::printf("cpptest: hotovo\n");
    return 0;
}
