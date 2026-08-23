% Un'animazione dentro CleeCode — Octave.
%
% Aprilo e premi ▶ Run, oppure manda una cella per volta con Ctrl+Shift+X.
%
% La cosa da sapere, ed è il motivo per cui c'è cleecode_frame() nel ciclo:
% il pannello e le schede si aggiornano quando Octave **aspetta un comando**.
% Durante un ciclo Octave non aspetta niente, quindi senza quella riga la
% scheda resta ferma al fotogramma di prima e si muove solo alla fine.
% cleecode_frame() ristampa le figure lì, in quel punto del ciclo.
%
% Fuori da CleeCode, e in una sessione impostata sulle finestre vere, non fa
% niente: quindi lo script continua a funzionare dovunque.
%
% Quanto va veloce, misurato su questa macchina — un fotogramma costa:
%
%     toolkit qt (Mac o Linux con schermo)      28 ms  (~30 al secondo)
%     gnuplot (server headless, via ssh)       148 ms  (~7 al secondo)
%
% Via ssh è più lenta e si vede lo stesso: il terminale riceve un PNG per
% fotogramma, non un flusso video. Se ti serve fluida su un collegamento
% sottile, disegna meno punti o allunga il pause.

%% onda che scorre
% Il caso base: un solo oggetto che cambia dati. Gli assi sono fissati con
% axis(), così l'animazione non fa ballare anche la cornice.
x = linspace(0, 2*pi, 300);
figure(1);
h = plot(x, sin(x), 'linewidth', 2);
axis([0 2*pi -1.2 1.2]);
grid on;
xlabel('x'); ylabel('sin(x - t)');
title('onda che scorre');

for k = 1:120
  set(h, 'ydata', sin(x - k/12));
  cleecode_frame();          % senza questa riga la scheda si muove solo alla fine
  pause(0.03);
end

%% una superficie che respira
% Più cara da stampare — 35 ms invece di 28 — ma è lo stesso ciclo.
[xx, yy] = meshgrid(linspace(-3, 3, 50));
base = peaks(50);
figure(2);
s = surf(xx, yy, base);
shading interp;
zlim([-10 10]);
title('respiro');

for k = 1:80
  set(s, 'zdata', base * (0.4 + 0.6 * sin(k/8)));
  cleecode_frame();
  pause(0.03);
end

%% un giro attorno alla superficie
% Ruotare è cambiare view(), e il ciclo è identico. Le stesse frecce sulla
% scheda della figura fanno questo a mano, 15° per volta.
for az = -180:4:180
  view(az, 30);
  cleecode_frame();
  pause(0.02);
end
