public class Sieve {
    public static void main(String[] a) {
        final int n = 10000000;
        long t0 = System.nanoTime();
        byte[] composite = new byte[n + 1];
        for (int i = 2; (long) i * i <= n; i++)
            if (composite[i] == 0)
                for (int j = i * i; j <= n; j += i) composite[j] = 1;
        long count = 0;
        for (int p = 2; p <= n; p++) if (composite[p] == 0) count++;
        System.out.println("CHECK " + count);
        System.out.println("MS " + (System.nanoTime() - t0) / 1000000);
    }
}
