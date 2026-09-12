"""Copy repository artwork and synthesize an original 110 BPM instrumental score."""
from pathlib import Path
import re, shutil, wave
import numpy as np
root=Path(__file__).resolve().parent
out=root/'public'; (out/'media').mkdir(parents=True,exist_ok=True)
for p in (root.parent/'docs/media').glob('*.webp'): shutil.copy2(p,out/'media'/p.name)
seconds=sum(map(int,re.findall(r'seconds:\s*(\d+)',(root/'src/scenes.ts').read_text())))
sr=44100
mix=np.zeros((seconds*sr,2),np.float64)
rng=np.random.default_rng(7)
def note(freq,start,duration,amp,pan=0,kind='pad'):
 n=int(duration*sr); t=np.arange(n)/sr
 if kind=='pad':
  env=np.minimum(t/.65,1)*np.minimum((duration-t)/1.1,1)
  sig=(np.sin(2*np.pi*freq*t)+.22*np.sin(2*np.pi*freq*2.002*t)+.12*np.sin(2*np.pi*freq*.998*t))*env*amp
 elif kind=='pluck': sig=(np.sin(2*np.pi*freq*t)+.24*np.sin(2*np.pi*freq*2*t))*np.exp(-t*4)*np.minimum(t/.008,1)*amp
 elif kind=='kick': sig=np.sin(2*np.pi*(47*t+4*(1-np.exp(-t*30))))*np.exp(-t*14)*amp
 elif kind=='hat':
  noise=rng.standard_normal(n);sig=(noise-np.roll(noise,1))*np.exp(-t*70)*np.minimum(t/.003,1)*amp
 a=int(start*sr); b=min(len(mix),a+n)
 if a>=len(mix):return
 gains=np.array([np.sqrt((1-pan)/2),np.sqrt((1+pan)/2)])
 mix[a:b]+=sig[:b-a,None]*gains
beat=60/110
chords=[[146.832,220,293.665,349.228],[130.813,195.998,261.626,329.628],[174.614,220,261.626,349.228],[97.999,146.832,195.998,293.665]]
for bar in range(int(seconds/(beat*8))+1):
 start=bar*beat*8;ch=chords[bar%4]
 for j,freq in enumerate(ch):note(freq,start,beat*8+1.2,.05,(-.4+j*.27))
 for j in range(16):
  at=start+j*beat/2
  if at<5 or at>seconds-6:continue
  note(ch[[0,2,1,3,2,1,3,2][j%8]]*2,at,1.4,.035 if j%4 else .055,np.sin(j)*.6,'pluck')
 for j in range(8):
  at=start+j*beat
  if 6<at<seconds-5:
   note(48,at,.4,.16,0,'kick')
   note(0,at+beat/2,.14,.026,.25,'hat')
# Gentle echoes make a continuous bed; no external or copyrighted samples.
delay=int(beat*.75*sr);mix[delay:]+=mix[:-delay].copy()*.16
fade=np.minimum(np.arange(len(mix))/sr/3,1)*np.minimum((len(mix)-np.arange(len(mix)))/sr/4,1)
mix*=fade[:,None];mix*=.78/max(np.max(np.abs(mix)),.001)
with wave.open(str(out/'score.wav'),'wb') as wav:
 wav.setnchannels(2);wav.setsampwidth(2);wav.setframerate(sr);wav.writeframes((mix*32767).astype('<i2').tobytes())
print(f'Prepared artwork and {seconds}s original stereo score; peak {np.max(np.abs(mix)):.2f}.')
