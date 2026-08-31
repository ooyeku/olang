public class Collatz {
    static long steps(long n) {
        long c = 0;
        while (n != 1) { n = n % 2 == 0 ? n / 2 : 3 * n + 1; c++; }
        return c;
    }
    public static void main(String[] a) {
        long t0 = System.nanoTime();
        long total = 0;
        for (long i = 1; i <= 300000; i++) total += steps(i);
        System.out.println("CHECK " + total);
        System.out.println("MS " + (System.nanoTime() - t0) / 1000000);
    }
}
