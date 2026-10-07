/*
 * pd-image-slots: copy one slot's boot and root partitions out of a whole
 * disk image streamed on stdin, without seeking.
 *
 *   xzcat image.img.xz | pd-image-slots --boot-out /dev/mmcblk0p3 \
 *       --root-out /dev/mmcblk0p6 [--boot-part 2] [--root-part 5] [--drain] \
 *       [--progress FILE]
 *
 * The image must use the A/B layout (docs/ota.md): boot slot A is primary
 * partition 2 (FAT), root slot A is logical partition 5 (Linux) inside the
 * extended partition. The MBR, each EBR and both partitions are visited in
 * ascending order as the stream passes them; an image that would need a
 * backwards read is refused. Each partition's bytes are written to its
 * output (a block device, or a file in tests) only after the output is known
 * to be large enough. --drain reads the stream to its end, for callers that
 * hash it on the way. --progress keeps FILE holding how far the copy is, in
 * thousandths (estimated until the root partition's end is known). On
 * success, prints BOOT_BYTES= and ROOT_BYTES= lines.
 *
 * Exit codes: 0 ok, 1 I/O error, 2 usage, 3 not the A/B layout, 4 too big.
 */
#define _FILE_OFFSET_BITS 64
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <linux/fs.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <unistd.h>

#define SECTOR 512u
#define CHUNK (1u << 20)

static uint64_t pos; /* sectors consumed from stdin */
static uint64_t total; /* sectors the copy needs, or an estimate */
static const char *progress_path;
static int progress_last = -1;
static unsigned char buf[CHUNK];

static void die(int code, const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	fputs("pd-image-slots: ", stderr);
	vfprintf(stderr, fmt, ap);
	fputc('\n', stderr);
	va_end(ap);
	exit(code);
}

/* Reads exactly n bytes; returns 0 at a clean end of stream before any. */
static size_t read_full(unsigned char *p, size_t n)
{
	size_t got = 0;
	while (got < n) {
		ssize_t r = read(0, p + got, n - got);
		if (r < 0 && errno == EINTR)
			continue;
		if (r < 0)
			die(1, "reading the image: %s", strerror(errno));
		if (r == 0)
			break;
		got += (size_t)r;
	}
	return got;
}

static void report(void)
{
	char tmp[4096];
	FILE *f;
	int pm;
	if (!progress_path || total == 0)
		return;
	pm = pos >= total ? 1000 : (int)(pos * 1000 / total);
	if (pm <= progress_last)
		return; /* only forward */
	progress_last = pm;
	snprintf(tmp, sizeof tmp, "%s.tmp", progress_path);
	f = fopen(tmp, "w");
	if (!f)
		return;
	fprintf(f, "%d\n", pm);
	if (fclose(f) == 0)
		rename(tmp, progress_path);
}

static void need(unsigned char *p, uint64_t sectors, const char *what)
{
	if (read_full(p, sectors * SECTOR) != sectors * SECTOR)
		die(3, "the image ends before %s", what);
	pos += sectors;
	report();
}

/* Discards input up to sector `to`. */
static void skip_to(uint64_t to, const char *what)
{
	if (to < pos)
		die(3, "%s comes before data already read; the layout needs seeking", what);
	while (pos < to) {
		uint64_t n = to - pos;
		if (n > CHUNK / SECTOR)
			n = CHUNK / SECTOR;
		need(buf, n, what);
	}
}

struct entry {
	unsigned type;
	uint64_t start, sectors; /* start relative to the table's base */
};

static struct entry entry_at(const unsigned char *table, int i)
{
	const unsigned char *e = table + 446 + 16 * i;
	struct entry out;
	out.type = e[4];
	out.start = (uint64_t)e[8] | (uint64_t)e[9] << 8 | (uint64_t)e[10] << 16 | (uint64_t)e[11] << 24;
	out.sectors = (uint64_t)e[12] | (uint64_t)e[13] << 8 | (uint64_t)e[14] << 16 | (uint64_t)e[15] << 24;
	return out;
}

static void check_signature(const unsigned char *s, const char *what)
{
	if (s[510] != 0x55 || s[511] != 0xaa)
		die(3, "%s has no partition table signature", what);
}

static int is_fat(unsigned t) { return t == 0x0b || t == 0x0c || t == 0x0e || t == 0x06; }
static int is_extended(unsigned t) { return t == 0x05 || t == 0x0f || t == 0x85; }

static uint64_t capacity(int fd, const char *path)
{
	struct stat st;
	uint64_t bytes = 0;
	const char *max;
	if (fstat(fd, &st) != 0)
		die(1, "%s: %s", path, strerror(errno));
	if (S_ISBLK(st.st_mode)) {
		if (ioctl(fd, BLKGETSIZE64, &bytes) != 0)
			die(1, "%s: %s", path, strerror(errno));
		return bytes;
	}
	/* Regular files (tests) grow, unless a limit is given. */
	max = getenv("PD_IMAGE_SLOTS_MAX_BYTES");
	return max ? strtoull(max, NULL, 10) : UINT64_MAX;
}

/* Copies `sectors` from the stream (at `start`) to `path`. */
static uint64_t copy_out(uint64_t start, uint64_t sectors, const char *path, const char *what)
{
	int fd;
	uint64_t left = sectors, bytes = sectors * SECTOR;
	struct stat st;

	skip_to(start, what);
	fd = open(path, O_WRONLY | O_CREAT, 0644);
	if (fd < 0)
		die(1, "%s: %s", path, strerror(errno));
	if (bytes > capacity(fd, path))
		die(4, "%s (%" PRIu64 " MiB) doesn't fit in %s", what, bytes >> 20, path);
	while (left > 0) {
		uint64_t n = left > CHUNK / SECTOR ? CHUNK / SECTOR : left;
		size_t off = 0, len = n * SECTOR;
		need(buf, n, what);
		while (off < len) {
			ssize_t w = write(fd, buf + off, len - off);
			if (w < 0 && errno == EINTR)
				continue;
			if (w < 0)
				die(1, "writing %s: %s", path, strerror(errno));
			off += (size_t)w;
		}
		left -= n;
	}
	if (fstat(fd, &st) == 0 && S_ISREG(st.st_mode) && ftruncate(fd, (off_t)bytes) != 0)
		die(1, "%s: %s", path, strerror(errno));
	if (fsync(fd) != 0 || close(fd) != 0)
		die(1, "%s: %s", path, strerror(errno));
	return bytes;
}

static void usage(void)
{
	die(2, "usage: pd-image-slots --boot-out PATH --root-out PATH "
	       "[--boot-part N] [--root-part N] [--drain] [--progress FILE] < image");
}

int main(int argc, char **argv)
{
	const char *boot_out = NULL, *root_out = NULL;
	int boot_part = 2, root_part = 5, drain = 0, i, logical;
	unsigned char mbr[SECTOR], ebr[SECTOR];
	struct entry boot = {0}, ext = {0};
	uint64_t boot_bytes = 0, root_bytes = 0, ebr_at;
	int boot_done = 0, root_done = 0;

	for (i = 1; i < argc; i++) {
		if (!strcmp(argv[i], "--drain"))
			drain = 1;
		else if (i + 1 >= argc)
			usage();
		else if (!strcmp(argv[i], "--boot-out"))
			boot_out = argv[++i];
		else if (!strcmp(argv[i], "--root-out"))
			root_out = argv[++i];
		else if (!strcmp(argv[i], "--boot-part"))
			boot_part = atoi(argv[++i]);
		else if (!strcmp(argv[i], "--root-part"))
			root_part = atoi(argv[++i]);
		else if (!strcmp(argv[i], "--progress"))
			progress_path = argv[++i];
		else
			usage();
	}
	if (!boot_out || !root_out || boot_part < 1 || boot_part > 4 || root_part < 5)
		usage();

	need(mbr, 1, "the partition table");
	check_signature(mbr, "the image");
	for (i = 0; i < 4; i++) {
		struct entry e = entry_at(mbr, i);
		if (i + 1 == boot_part)
			boot = e;
		if (is_extended(e.type))
			ext = e;
	}
	if (!is_fat(boot.type) || boot.sectors == 0 || ext.sectors == 0)
		die(3, "this image doesn't use the A/B layout (no FAT boot partition %d "
		       "and extended partition)", boot_part);

	/* Until root A's end is known, guess it lies a few boot slots past the
	 * extended partition's start (a 512 MiB root after a 128 MiB boot). */
	total = ext.start + 4 * boot.sectors;

	/* Walk the EBR chain in stream order, copying boot A when we pass it. */
	ebr_at = ext.start;
	for (logical = 5; !root_done; logical++) {
		struct entry part, next;
		if (!boot_done && boot.start < ebr_at) {
			boot_bytes = copy_out(boot.start, boot.sectors, boot_out, "boot slot A");
			boot_done = 1;
		}
		skip_to(ebr_at, "an extended boot record");
		need(ebr, 1, "an extended boot record");
		check_signature(ebr, "an extended boot record");
		part = entry_at(ebr, 0);
		next = entry_at(ebr, 1);
		if (part.sectors != 0 && logical == root_part) {
			if (part.type != 0x83)
				die(3, "partition %d isn't a Linux root partition", root_part);
			total = ebr_at + part.start + part.sectors;
			if (!boot_done && boot.start < ebr_at + part.start) {
				boot_bytes = copy_out(boot.start, boot.sectors, boot_out, "boot slot A");
				boot_done = 1;
			}
			root_bytes = copy_out(ebr_at + part.start, part.sectors, root_out, "root slot A");
			root_done = 1;
			continue;
		}
		if (part.sectors == 0)
			logical--; /* Linux numbers only non-empty logical partitions */
		if (next.sectors == 0)
			die(3, "this image has no partition %d (root slot A)", root_part);
		ebr_at = ext.start + next.start;
	}
	if (!boot_done)
		boot_bytes = copy_out(boot.start, boot.sectors, boot_out, "boot slot A");

	if (drain)
		while (read_full(buf, CHUNK) > 0)
			;
	printf("BOOT_BYTES=%" PRIu64 "\nROOT_BYTES=%" PRIu64 "\n", boot_bytes, root_bytes);
	return 0;
}
