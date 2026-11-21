/*
 * Runs a command and reports its wall time and resource usage on file
 * descriptor FD:
 *
 *     spawn-measure FD PROGRAM [ARG...]
 *     -> "<wall ns> <user us> <sys us> <max rss KiB> <minor faults> <major faults> <wait status>\n"
 *
 * Used by scripts/time-commands.py. Linux charges a process's peak RSS from
 * before exec to the program it execs, so a child forked from a Python
 * process of 20 MiB reports at least 20 MiB, however small it is. Forked
 * from this program instead, it reports its own.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    if (argc < 3) {
        fprintf(stderr, "usage: spawn-measure FD PROGRAM [ARG...]\n");
        return 2;
    }
    int fd = atoi(argv[1]);
    FILE *report = fdopen(fd, "w");
    if (report == NULL) {
        perror("spawn-measure: report descriptor");
        return 2;
    }
    fcntl(fd, F_SETFD, FD_CLOEXEC);

    struct timespec start, end;
    clock_gettime(CLOCK_MONOTONIC, &start);
    pid_t pid = fork();
    if (pid < 0) {
        perror("spawn-measure: fork");
        return 2;
    }
    if (pid == 0) {
        execv(argv[2], argv + 2);
        perror("spawn-measure: exec");
        _exit(127);
    }
    int status;
    struct rusage usage;
    if (wait4(pid, &status, 0, &usage) < 0) {
        perror("spawn-measure: wait4");
        return 2;
    }
    clock_gettime(CLOCK_MONOTONIC, &end);

    long long wall = (long long)(end.tv_sec - start.tv_sec) * 1000000000LL
                   + (end.tv_nsec - start.tv_nsec);
    fprintf(report, "%lld %lld %lld %ld %ld %ld %d\n", wall,
            (long long)usage.ru_utime.tv_sec * 1000000 + usage.ru_utime.tv_usec,
            (long long)usage.ru_stime.tv_sec * 1000000 + usage.ru_stime.tv_usec,
            usage.ru_maxrss, usage.ru_minflt, usage.ru_majflt, status);
    fclose(report);
    return 0;
}
